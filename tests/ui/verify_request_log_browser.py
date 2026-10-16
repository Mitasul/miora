#!/usr/bin/env python3
"""Real request-log drawer layout regression using an isolated local upstream.

Run after cargo build. Requires Playwright Python and a local Chrome executable.
No production configuration, external supplier, API mock or CSS override is used.
"""
from __future__ import annotations
import argparse
import http.server
import json
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tests/http'))
import verify_request_logs as fixture
from http_fixture_process import wait_for_gateway, stop_gateway


def check(condition, message):
    if not condition:
        raise AssertionError(message)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', default='target/debug/uni-api-front.exe')
    parser.add_argument('--chrome', default=r'C:\Program Files\Google\Chrome\Application\chrome.exe')
    parser.add_argument('--artifacts', default='.artifacts/admin-ui-modern/logs')
    parser.add_argument('--color-scheme', choices=['light', 'dark'], default='light')
    args = parser.parse_args()
    from playwright.sync_api import sync_playwright, expect
    binary = (ROOT / args.binary).resolve()
    check(binary.is_file(), f'Build binary first: {binary}')
    output = (ROOT / args.artifacts).resolve()
    output.mkdir(parents=True, exist_ok=True)
    report = {'passed': False, 'backend': 'isolated real Rust gateway with SQLite',
              'api_mocks': False, 'asset_overrides': False, 'checks': [], 'screenshots': []}
    # This string is returned by the test-only upstream, never injected into UI markup.
    original_svg = fixture.SVG
    fixture.SVG = '<svg onload="window.__requestLogSvgExecuted=1"><text>untrusted fixture</text></svg>' + ' fixture-text-' * 12000
    upstream = http.server.ThreadingHTTPServer(('127.0.0.1', 0), fixture.Upstream)
    upstream.hits = []
    threading.Thread(target=upstream.serve_forever, daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix='miora-log-ui-') as directory:
            root = Path(directory)
            executable = root / ('gateway.exe' if os.name == 'nt' else 'gateway')
            shutil.copy2(binary, executable)
            config = {'providers': [{'provider': 'log-ui-fixture', 'engine': 'gpt',
                      'base_url': f'http://127.0.0.1:{upstream.server_port}/v1/chat/completions',
                      'api': 'fixture-provider-secret', 'model': [{'upstream-model': 'public-model'}]}],
                      'api_keys': [{'api': 'fixture-caller-secret', 'role': 'user', 'model': ['all']}]}
            (root / 'api.json').write_text(json.dumps(config), encoding='utf-8')
            port = fixture.free_port()
            origin = f'http://127.0.0.1:{port}'
            password = secrets.token_urlsafe(24)
            env = {key: value for key, value in os.environ.items()
                   if not key.startswith(('MIORA_', 'FACTS_S3_', 'UNI_API_CONTROL_RESTORE')) and key != 'CONFIG_URL'}
            env.update(PORT=str(port), DISABLE_DATABASE='false', DB_TYPE='sqlite', DB_PATH=str(root / 'stats.db'),
                       MIORA_ADMIN_USERNAME='admin', MIORA_ADMIN_PASSWORD=password, MIORA_ADMIN_ORIGIN=origin,
                       MIORA_ADMIN_SECURE_COOKIE='false', UNI_API_CONFIG_PATH=str(root / 'api.json'),
                       RUST_RESPONSES_CONFIG_SNAPSHOT_PATH=str(root / 'snapshot.json'),
                       UNI_API_SHARED_MEMORY_RESERVATION_PATH=str(root / 'ledger'),
                       RUST_REQUEST_SPOOL_DIRECTORY=str(root / 'spool'), MIORA_PLUGIN_STORE_DIR=str(root / 'plugins'),
                       MIORA_REQUEST_LOG_DIR=str(root / 'request-logs'), NO_PROXY='127.0.0.1,localhost')
            with (root / 'runtime.log').open('wb') as log:
                process = subprocess.Popen([str(executable)], cwd=root, env=env, stdout=log, stderr=log, creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
                try:
                    wait_for_gateway(process, port)
                    client = fixture.Client(origin)
                    status, headers, session, _ = client.request('POST', '/admin/api/login', {'username': 'admin', 'password': password})
                    check(status == 200, 'Fixture administrator login failed')
                    client.cookie = headers['set-cookie'].split(';', 1)[0]
                    client.csrf = session['csrf_token']

                    def api(method, path, data=None):
                        result = client.request(method, '/admin/api' + path, data, headers={'X-CSRF-Token': client.csrf})
                        check(result[0] == 200, f'{method} {path}: HTTP {result[0]}')
                        return result[2]

                    settings = api('GET', '/log-settings')
                    settings['settings'].update(enabled=True, capture_fields=fixture.ALL_FIELDS, body_retention_hours=24,
                                                limits={'per_request_bytes': 1048576, 'total_body_bytes': 8388608})
                    api('PUT', '/log-settings', {'revision': settings['revision'], 'settings': settings['settings']})
                    request_id = 'ab' * 16
                    status, _, _, _ = client.request('POST', '/v1/chat/completions',
                        {'model': 'public-model', 'messages': [{'role': 'user', 'content': 'visual fixture'}]},
                        headers={'Authorization': 'Bearer fixture-caller-secret', 'X-Request-ID': request_id}, authenticated=False)
                    check(status == 200, 'Local upstream fixture failed')
                    rows = []
                    deadline = time.monotonic() + 10
                    while time.monotonic() < deadline:
                        rows = api('GET', '/request-logs?' + urllib.parse.urlencode({'request_id': request_id}))['rows']
                        if rows and not rows[0]['active']:
                            break
                        time.sleep(.05)
                    check(rows and not rows[0]['active'], 'Request log did not finish')
                    record = api('GET', '/request-logs/' + rows[0]['id'])
                    check(all(any(meta['field'] == field and meta['available'] for meta in record['fields'])
                              for field in fixture.ALL_FIELDS), 'Real fixture is missing captured stages')
                    report['checks'].append('real local inference and all eight captured fields available')
                    with sync_playwright() as playwright:
                        browser = playwright.chromium.launch(executable_path=args.chrome, headless=True)
                        try:
                            context = browser.new_context(viewport={'width': 1440, 'height': 1000}, locale='zh-CN', reduced_motion='reduce', color_scheme=args.color_scheme)
                            name, value = client.cookie.split('=', 1)
                            context.add_cookies([{'name': name, 'value': value, 'url': origin, 'httpOnly': True, 'sameSite': 'Strict'}])
                            check(context.request.get(origin + '/admin/assets/styles.css').body() ==
                                  (ROOT / 'assets/admin/styles.css').read_bytes(), 'Embedded stylesheet is stale')
                            report['checks'].append('actual embedded stylesheet matches working-tree bytes')
                            page = context.new_page()
                            page.set_default_timeout(20000)
                            errors, csp = [], []
                            page.on('pageerror', lambda error: errors.append(str(error)))
                            page.on('console', lambda message: csp.append(message.text) if
                                    'Content Security Policy' in message.text or 'violates the following' in message.text else None)

                            def shot(name):
                                # A top-layer dialog is viewport-fixed. Full-page captures can
                                # incorrectly place it at the document's pre-open scroll offset.
                                page.screenshot(path=str(output / name), full_page=not page.locator('#editor-dialog').is_visible())
                                report['screenshots'].append(name)

                            def route(name):
                                page.goto(origin + '/admin#' + name)
                                expect(page.locator('#main-content')).not_to_have_attribute('aria-busy', 'true')
                                check(page.locator('.error-state').count() == 0, f'{name}: failed to load')
                                check(page.evaluate('document.documentElement.scrollWidth <= innerWidth + 1'), f'{name}: viewport overflow')

                            def drawer_layout():
                                bounds = page.evaluate('''() => {
                                    const form = document.querySelector('#editor-form');
                                    const modal = document.querySelector('#editor-dialog').getBoundingClientRect();
                                    const tabs = form.querySelector('.request-log-tabs');
                                    const body = form.querySelector('.dialog-body');
                                    const rect = node => node.getBoundingClientRect();
                                    const t = rect(tabs), b = rect(body), h = rect(form.querySelector('.dialog-head'));
                                    const f = rect(form.querySelector('.form-footer'));
                                    return {separate: tabs.parentElement === form,
                                            fitsViewport: modal.left >= -1 && modal.top >= -1 && modal.right <= innerWidth + 1 && modal.bottom <= innerHeight + 1,
                                            ordered: h.bottom <= t.top + 1 && t.bottom <= b.top + 1 && b.bottom <= f.top + 1,
                                            footerVisible: f.bottom <= innerHeight + 1,
                                            horizontal: body.scrollWidth <= body.clientWidth + 1};
                                }''')
                                check(all(bounds.values()), f'Drawer layout regression: {bounds}')

                            for size, width, height in [('desktop', 1440, 1000), ('mobile', 390, 844)]:
                                page.set_viewport_size({'width': width, 'height': height})
                                route('log-settings')
                                shot(f'log-settings-enabled-{size}.png')
                                route('logs')
                                expect(page.locator('[data-request-log-action="detail"]')).to_have_count(1)
                                shot(f'request-logs-{size}.png')
                                page.locator('[data-request-log-action="detail"]').click()
                                expect(page.locator('[data-log-tab="overview"]')).to_have_attribute('aria-selected', 'true')
                                for tab in ['overview', 'request', 'upstream-request', 'upstream-response', 'response', 'attempts']:
                                    page.locator(f'[data-log-tab="{tab}"]').click()
                                    drawer_layout()
                                    if tab == 'response':
                                        page.locator('[data-log-content]').last.click()
                                        expect(page.locator('pre.log-content-text')).to_be_visible()
                                        check('<svg onload=' in page.locator('pre.log-content-text').inner_text(), 'Expected untrusted fixture as text')
                                        check(page.locator('pre.log-content-text svg').count() == 0 and
                                              page.evaluate('window.__requestLogSvgExecuted === undefined'), 'Untrusted content executed')
                                        expect(page.get_by_text('预览长度受限，以下不是完整已保存内容。需要更多内容时，可下载服务端当前仍可用的存储文件。', exact=True)).to_be_visible()
                                        page.locator('.dialog-body').evaluate('(node) => { node.scrollTop = 0; }')
                                        drawer_layout()
                                        shot(f'request-log-response-top-{size}.png')
                                        before = page.locator('.request-log-tabs').bounding_box()
                                        page.locator('.dialog-body').evaluate('(node) => { node.scrollTop = node.scrollHeight; }')
                                        drawer_layout()
                                        check(page.locator('.request-log-tabs').bounding_box() == before, 'Scrolling moved or obscured drawer tabs')
                                        shot(f'request-log-response-content-{size}.png')
                                    else:
                                        shot(f'request-log-{tab}-{size}.png')
                                page.keyboard.press('Escape')
                                expect(page.locator('#editor-dialog')).not_to_be_visible()
                            check(not errors and not csp, f'Browser errors: {errors}; CSP: {csp}')
                            report['checks'] += ['desktop and 390px enabled settings/list fit viewport',
                                'six real request-log tabs render at both sizes without horizontal overflow',
                                'drawer header/tabs/footer stay outside scrolling body with no overlap',
                                'long untrusted SVG preview stays text; truncation is disclosed',
                                'zero JavaScript exceptions or CSP violations']
                            report['passed'] = True
                        finally:
                            browser.close()
                finally:
                    stop_gateway(process)
    except Exception as error:
        report['failure'] = str(error)
        raise
    finally:
        fixture.SVG = original_svg
        upstream.shutdown()
        upstream.server_close()
        (output / 'browser-report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
    print(f'PASS: real request-log browser regression, {len(report["checks"])} checks; screenshots: {output}')


if __name__ == '__main__':
    main()
