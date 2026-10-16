#!/usr/bin/env python3
"""Real-browser regression against an isolated, actual Rust gateway (no API mocks).

Requires Playwright Python and an installed Chromium/Chrome executable.
Run after cargo build (assets are include_str! embedded):
  python tests/ui/verify_admin_browser.py --binary target/debug/uni-api-front.exe
Optional: --chrome PATH --artifacts .artifacts/admin-ui
All source configuration, credentials and gateway state are temporary. Screenshots
never contain passwords, upstream secrets or the one-time API Key value.
"""
from __future__ import annotations
import argparse
import importlib.util
import threading
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import tempfile
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[2]


def check(condition, message):
    if not condition:
        raise AssertionError(message)


def free_port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', default='target/debug/uni-api-front.exe')
    parser.add_argument('--chrome', default=r'C:\Program Files\Google\Chrome\Application\chrome.exe')
    parser.add_argument('--artifacts', default='.artifacts/admin-ui')
    parser.add_argument('--color-scheme', choices=['light', 'dark'], default='light')
    args = parser.parse_args()
    binary = (ROOT / args.binary).resolve()
    check(binary.is_file(), f'Build binary first: {binary}')
    check(Path(args.chrome).is_file(), f'Browser executable not found: {args.chrome}')
    from playwright.sync_api import sync_playwright, expect
    fixture_spec = importlib.util.spec_from_file_location('plugin_fixture', ROOT / 'tests/http/verify_channel_plugins.py')
    fixture = importlib.util.module_from_spec(fixture_spec)
    fixture_spec.loader.exec_module(fixture)
    artifacts = (ROOT / args.artifacts).resolve()
    artifacts.mkdir(parents=True, exist_ok=True)
    report = {'backend': 'isolated real Rust gateway', 'api_mocks': False, 'checks': [], 'screenshots': []}
    with tempfile.TemporaryDirectory(prefix='miora-ui-', dir=artifacts) as directory:
        temp = Path(directory)
        origin = f'http://127.0.0.1:{free_port()}'
        password = 'UiRegression-' + secrets.token_hex(20)
        env = {k: v for k, v in os.environ.items() if not k.startswith(('MIORA_ADMIN_', 'MIORA_PLUGIN_', 'FACTS_S3_')) and k not in ('CONFIG_URL', 'MIORA_TEST_USER_BEARER')}
        env.update(PORT=origin.rsplit(':', 1)[1], DISABLE_DATABASE='true', MIORA_BOOTSTRAP_EMPTY_CONFIG='true', UNI_API_CONFIG_PATH=str(temp / 'api.yaml'), RUST_RESPONSES_CONFIG_SNAPSHOT_PATH=str(temp / 'snapshot.json'), UNI_API_SHARED_MEMORY_RESERVATION_PATH=str(temp / 'ledger'), RUST_REQUEST_SPOOL_DIRECTORY=str(temp / 'spool'), RUST_REQUEST_SPOOL_DISK_RESERVE_BPS='0', RUST_REQUEST_SPOOL_INODE_RESERVE_BPS='0', MIORA_PLUGIN_STORE_DIR=str(temp / 'plugins'), MIORA_ADMIN_USERNAME='admin', MIORA_ADMIN_PASSWORD=password, MIORA_ADMIN_SECURE_COOKIE='false', MIORA_ADMIN_ORIGIN=origin, NO_PROXY='127.0.0.1,localhost')
        service = fixture.echo.ThreadingHTTPServer(('127.0.0.1', 0), fixture.Service)
        service.token = 'ui-fixture-control-token'
        service.hits = []
        threading.Thread(target=service.serve_forever, daemon=True).start()
        env['MIORA_PLUGIN_ALLOW_LOOPBACK_HTTP'] = 'true'
        with (temp / 'gateway.log').open('w+', encoding='utf-8') as log:
            process = subprocess.Popen([str(binary)], cwd=temp, env=env, stdout=log, stderr=log, creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
            try:
                ready = False
                for _ in range(300):
                    if process.poll() is not None:
                        break
                    try:
                        with urllib.request.urlopen(origin + '/healthz', timeout=1) as response:
                            ready = response.status == 200
                        if ready:
                            break
                    except OSError:
                        pass
                    time.sleep(.1)
                check(ready, f'Gateway did not start (exit={process.poll()})')
                for name in ['app.js', 'theme.js', 'ui.js', 'api.js', 'styles.css', 'login.js', 'icons.js', 'plugin-manager.js', 'models-editor.js', 'channel-editor.js']:
                    path = '/admin' if name == 'index.html' else '/admin/assets/' + name
                    with urllib.request.urlopen(origin + path, timeout=10) as response:
                        actual = response.read()
                    expected = (ROOT / 'assets/admin' / name).read_bytes()
                    check(actual == expected, f'Embedded {name} is stale; rebuild Rust binary before browser regression')
                report['checks'].append('embedded assets match working-tree bytes')
                with sync_playwright() as playwright:
                    browser = playwright.chromium.launch(executable_path=args.chrome, headless=True)
                    context = browser.new_context(viewport={'width': 1440, 'height': 1000}, locale='zh-CN', reduced_motion='reduce', color_scheme=args.color_scheme)
                    page = context.new_page()
                    page_errors = []
                    csp_errors = []
                    page.on('pageerror', lambda error: page_errors.append(str(error)))
                    page.on('console', lambda message: csp_errors.append(message.text) if 'Content Security Policy' in message.text or 'violates the following' in message.text else None)
                    page.set_default_timeout(20000)

                    def shot(name):
                        expect(page.locator('.toast')).to_have_count(0, timeout=8000)
                        full_page = name != 'navigation-mobile.png' and not page.locator('#editor-dialog').is_visible()
                        page.screenshot(path=str(artifacts / name), full_page=full_page)
                        report['screenshots'].append(name)

                    def check_channel_layout():
                        bounds = page.evaluate('''() => {
                            const modal = document.querySelector('#editor-dialog');
                            const form = modal.querySelector('#editor-form');
                            const modalBounds = modal.getBoundingClientRect();
                            const rect = selector => form.querySelector(selector).getBoundingClientRect();
                            const head = rect('.dialog-head'), tabs = rect('.channel-tabs');
                            const body = rect('.dialog-body'), footer = rect('.form-footer');
                            return {horizontal: modal.scrollWidth <= modal.clientWidth + 1,
                                    fitsViewport: modalBounds.left >= -1 && modalBounds.top >= -1 && modalBounds.right <= innerWidth + 1 && modalBounds.bottom <= innerHeight + 1,
                                    bodyHorizontal: form.querySelector('.dialog-body').scrollWidth <= body.width + 1,
                                    ordered: head.bottom <= tabs.top + 1 && tabs.bottom <= body.top + 1 && body.bottom <= footer.top + 1,
                                    footerVisible: footer.bottom <= innerHeight + 1};
                        }''')
                        check(all(bounds.values()), f'Channel dialog layout regression: {bounds}')

                    def route(name):
                        page.goto(origin + '/admin#' + name)
                        expect(page.locator('#main-content')).not_to_have_attribute('aria-busy', 'true')
                        expect(page.locator('#session-state')).to_have_text('管理会话已验证')
                        nav_name = 'settings' if name == 'log-settings' else 'logs' if name == 'usage' else name
                        expect(page.locator(f'[data-route="{nav_name}"]')).to_have_attribute('aria-current', 'page')
                        check(page.locator('.error-state').count() == 0, f'{name}: page error: {page.locator("#main-content").inner_text()}')

                    def submit(method, suffix):
                        with page.expect_response(lambda response: response.request.method == method and response.url.split('?')[0].endswith('/admin/api' + suffix)) as pending:
                            page.locator('#editor-form button[type="submit"]').click()
                        response = pending.value
                        check(response.ok, f'{method} {suffix}: {response.status}: {response.text()}')
                        expect(page.locator('#editor-dialog')).not_to_be_visible()
                        expect(page.locator('#main-content')).not_to_have_attribute('aria-busy', 'true')

                    page.goto(origin + '/login')
                    expect(page.locator('#login-title')).to_have_text('登录')
                    check(page.locator('img.brand-mark').evaluate('(image) => image.complete && image.naturalWidth > 0'), 'Login repository logo did not load')
                    shot('login-desktop.png')
                    page.set_viewport_size({'width': 390, 'height': 844})
                    check(page.evaluate('document.documentElement.scrollWidth <= innerWidth + 1'), 'Narrow login overflows viewport')
                    shot('login-mobile.png')
                    for width in [320, 768, 1024, 1920]:
                        page.set_viewport_size({'width': width, 'height': 1000})
                        page.wait_for_function('width => innerWidth === width && document.documentElement.scrollWidth <= width + 1', arg=width)
                        check(page.evaluate('document.documentElement.scrollWidth <= innerWidth + 1'), f'Login at {width}px overflows viewport')
                    page.set_viewport_size({'width': 1440, 'height': 1000})
                    page.locator('#password').fill(password)
                    page.locator('#login-submit').click()
                    page.wait_for_url('**/admin#overview')
                    expect(page.locator('#session-state')).to_have_text('管理会话已验证')
                    expect(page.locator('#main-content')).not_to_have_attribute('aria-busy', 'true')
                    check(context.request.get(origin + '/admin').body() == (ROOT / 'assets/admin/index.html').read_bytes(), 'Embedded index.html is stale; rebuild binary')
                    cookies = context.cookies(origin + '/admin')
                    check(any(cookie['httpOnly'] and cookie['sameSite'] == 'Strict' for cookie in cookies), 'Session cookie flags missing')
                    check(page.evaluate('localStorage.length === 0 && sessionStorage.length === 0'), 'UI persisted browser storage')
                    report['checks'].append('real password login, HttpOnly/SameSite session, no token storage')
                    shot('overview-desktop-empty.png')
                    check(page.locator('img.brand-mark').evaluate('(image) => image.complete && image.naturalWidth > 0'), 'Sidebar repository logo did not load')
                    geometry = page.evaluate('''() => Object.fromEntries(
                        ['.card', '.overview-inventory .badge', '.button', '#primary-nav a'].map(selector => {
                            const style = getComputedStyle(document.querySelector(selector));
                            return [selector, parseFloat(style.borderTopLeftRadius)];
                        }))''')
                    check(all(radius <= 4 for radius in geometry.values()), f'Excessive corner radii: {geometry}')
                    report['checks'].append('repository brand loads on login/sidebar; shared controls use corners of at most 4px')
                    for width in [320, 768, 1024, 1920]:
                        page.set_viewport_size({'width': width, 'height': 1000})
                        page.wait_for_function('width => innerWidth === width && document.documentElement.scrollWidth <= width + 1', arg=width)
                        layout = page.evaluate('''() => ({width: innerWidth, scrollWidth: document.documentElement.scrollWidth,
                            overflow: [...document.querySelectorAll('#main-content *, .topbar *, .app-footer *')]
                                .filter(element => element.getBoundingClientRect().right > innerWidth + 1)
                                .slice(0, 8).map(element => ({tag: element.tagName, class: element.className,
                                    right: element.getBoundingClientRect().right}))})''')
                        check(layout['scrollWidth'] <= width + 1, f'Overview at {width}px overflows viewport: {layout}')
                        shot(f'overview-{width}px.png')
                        check(page.locator('.overview-inventory .badge').count() == 3, f'Overview at {width}px lost inventory')
                    page.set_viewport_size({'width': 1440, 'height': 1000})
                    report['checks'].append('login and merged overview metrics fit 320px, 768px, 1024px and 1920px widths')
                    for name in ['overview', 'channels', 'plugins', 'interceptors', 'keys', 'logs', 'log-settings', 'usage', 'settings']:
                        route(name)
                        check(page.locator('#main-content h1').count() == 1, f'{name}: missing page heading')
                        check(page.evaluate('document.documentElement.scrollWidth <= innerWidth + 1'), f'Desktop {name} overflows viewport')
                        shot(f'{name}-desktop.png')
                    report['checks'].append('all nine real-backend hash routes render without error states or desktop overflow')

                    route('channels')
                    expect(page.get_by_text('还没有配置渠道', exact=True)).to_be_visible()
                    page.locator('[data-action="create-channel"]').click()
                    expect(page.locator('#editor-dialog')).to_be_visible()
                    for tab in ['basic', 'models', 'parameters']:
                        page.locator(f'[data-channel-tab="{tab}"]').click()
                        check_channel_layout()
                        shot(f'channel-{tab}-desktop.png')
                    page.locator('[data-channel-tab="basic"]').click()
                    page.locator('#f-provider').fill('ui-regression-channel')
                    page.locator('#f-base_url').fill('https://example.invalid/v1?api_key=ui-query-secret')
                    page.locator('#f-api').fill('upstream-test-' + secrets.token_hex(12))
                    page.locator('[data-channel-tab="models"]').click()
                    page.locator('#manual-model-input').fill('ui-model-a')
                    page.locator('[data-model-action="add"]').click()
                    expect(page.locator('.model-selected-row [data-upstream-index="0"]')).to_have_value('ui-model-a')
                    page.locator('[data-price-row="0"] summary').click()
                    page.locator('[data-price-index="0"][data-price-key="input"]').fill('2')
                    page.locator('[data-price-index="0"][data-price-key="output"]').fill('8')
                    page.locator('[data-price-index="0"][data-price-key="cache_read"]').fill('0.2')
                    submit('POST', '/channels')
                    expect(page.get_by_text('ui-regression-channel', exact=True)).to_be_visible()
                    shot('channels-desktop.png')
                    page.locator('[data-action="edit-channel"]').click()
                    expect(page.locator('#f-api')).to_have_value('')
                    page.locator('[data-channel-tab="models"]').click()
                    expect(page.locator('[data-price-index="0"][data-price-key="input"]')).to_have_value('2')
                    page.locator('#manual-model-input').fill('ui-model-b')
                    page.locator('[data-model-action="add"]').click()
                    expect(page.locator('.model-selected-row')).to_have_count(2)
                    expect(page.locator('.model-selected-row [data-upstream-index="1"]')).to_have_value('ui-model-b')
                    page.locator('[data-alias-index="1"]').fill('public-b')
                    page.locator('[data-price-row="0"] summary').click()
                    page.locator('[data-price-row="1"] summary').click()
                    page.locator('[data-price-index="1"][data-price-key="input"]').fill('4')
                    page.locator('[data-price-index="1"][data-price-key="output"]').fill('16')
                    page.locator('[data-price-index="1"][data-price-key="cache_read"]').fill('0')
                    check_channel_layout()
                    page.locator('.model-selected-row').first.scroll_into_view_if_needed()
                    shot('model-pricing-desktop.png')
                    for width in [320, 390]:
                        page.set_viewport_size({'width': width, 'height': 844})
                        check_channel_layout()
                        page.locator('.model-selected-row').first.scroll_into_view_if_needed()
                        shot(f'model-pricing-{width}px.png')
                    page.set_viewport_size({'width': 1440, 'height': 1000})
                    submit('PATCH', '/channels')
                    expect(page.get_by_text('public-b', exact=True)).to_be_visible()
                    detail = page.request.get(origin + '/admin/api/channels/detail?provider=ui-regression-channel').json()
                    prices = detail['effective']['preferences']['usage_pricing']
                    check(prices == {'models': {'ui-model-a': {'input': 2, 'output': 8, 'cache_read': .2}, 'public-b': {'input': 4, 'output': 16, 'cache_read': 0}}}, f'Model prices crossed or used upstream rather than public names: {prices}')
                    page.locator('[data-action="edit-channel"]').click()
                    page.locator('[data-channel-tab="models"]').click()
                    expect(page.locator('[data-price-index="0"][data-price-key="input"]')).to_have_value('2')
                    expect(page.locator('[data-price-index="1"][data-price-key="input"]')).to_have_value('4')
                    page.locator('[data-alias-index="1"]').fill('renamed-b')
                    submit('PATCH', '/channels')
                    detail = page.request.get(origin + '/admin/api/channels/detail?provider=ui-regression-channel').json()
                    prices = detail['effective']['preferences']['usage_pricing']['models']
                    check('public-b' not in prices and prices['renamed-b']['input'] == 4, 'Renaming a model did not move its prices')
                    page.locator('[data-action="edit-channel"]').click()
                    page.locator('[data-channel-tab="models"]').click()
                    page.locator('[data-model-action="remove"][data-row-index="1"]').click()
                    submit('PATCH', '/channels')
                    detail = page.request.get(origin + '/admin/api/channels/detail?provider=ui-regression-channel').json()
                    check(detail['effective']['preferences']['usage_pricing']['models'] == {'ui-model-a': {'input': 2, 'output': 8, 'cache_read': .2}}, 'Removed model left behind stale prices or changed another model')
                    report['checks'].append('per-model prices create/edit/reopen independently, follow public aliases, and fit 320px/390px/desktop')
                    check('ui-query-secret' in (temp / 'api.yaml').read_text(encoding='utf-8'), 'Editing models overwrote the untouched secret-bearing source URL')
                    page.locator('[data-action="copy-channel"]').click()
                    expect(page.locator('#dialog-title')).to_have_text('复制渠道')
                    page.locator('#f-copy_name').fill('ui-regression-copy')
                    submit('POST', '/channels')
                    expect(page.locator('tr[data-search]')).to_have_count(2)
                    saved = json.loads((temp / 'snapshot.json').read_text(encoding='utf-8'))['api_config']['providers']
                    original = next(channel for channel in saved if channel['provider'] == 'ui-regression-channel')
                    copied = next(channel for channel in saved if channel['provider'] == 'ui-regression-copy')
                    check({**copied, 'provider': original['provider']} == original, 'Copy changed source credentials, secret URL, model aliases or prices')
                    check('ui-query-secret' not in page.locator('#main-content').inner_text(), 'Copied channel leaked secret URL into the list')
                    usage_requests = []
                    page.on('request', lambda request: usage_requests.append(request.url) if request.url.split('?')[0].endswith('/admin/api/channels') else None)
                    page.locator('#channel-search').fill('ui-regression-copy')
                    expect(page.locator('tr[data-search]:visible')).to_have_count(1)
                    page.locator('#f-channel_type_filter').select_option('gpt')
                    expect(page.locator('tr[data-search]:visible')).to_have_count(1)
                    check(not usage_requests, 'Channel filtering issued a network request')
                    page.locator('tr[data-search]:visible [data-action="delete-channel"]').click()
                    submit('DELETE', '/channels')
                    report['checks'].append('channel copy preserves exact source secrets/prices/aliases; local keyword and engine filters make no requests')
                    page.locator('[data-action="delete-channel"]').click()
                    expect(page.locator('#dialog-title')).to_have_text('删除渠道')
                    submit('DELETE', '/channels')
                    expect(page.get_by_text('还没有配置渠道', exact=True)).to_be_visible()
                    report['checks'].append('channel empty state, create/edit/delete, secret not echoed, untouched URL secret preserved, dangerous confirmation')

                    route('keys')
                    page.locator('[data-action="create-key"]').click()
                    expect(page.locator('#f-role')).to_have_value('user')
                    with page.expect_response(lambda response: response.request.method == 'POST' and response.url.endswith('/admin/api/api-keys')) as pending:
                        page.locator('#editor-form button[type="submit"]').click()
                    check(pending.value.ok, f'Key creation failed ({pending.value.status})')
                    expect(page.locator('#dialog-title')).to_have_text('API Key 创建成功')
                    expect(page.locator('#f-new_key')).to_have_value(__import__('re').compile('.{20,}'))
                    page.locator('#editor-form button[type="submit"]').click()
                    expect(page.locator('#editor-dialog')).not_to_be_visible()
                    expect(page.locator('#f-new_key')).to_have_count(0)
                    page.locator('[data-action="edit-key"]').click()
                    page.locator('#f-models').fill('ui-model-a')
                    submit('PATCH', '/api-keys')
                    page.locator('[data-action="revoke-key"]').click()
                    submit('DELETE', '/api-keys')
                    expect(page.get_by_text('没有配置 API Key', exact=True)).to_be_visible()
                    check(page.evaluate('localStorage.length === 0 && sessionStorage.length === 0'), 'Key stored in web storage')
                    report['checks'].append('key create (default user), one-time display destroyed on close, edit, revoke')

                    route('interceptors')
                    page.locator('[data-action="create-interceptor"]').click()
                    page.locator('#f-id').fill('ui-rule')
                    page.locator('#f-set_headers').fill('x-ui-regression: yes')
                    submit('PUT', '/interceptors')
                    expect(page.get_by_text('ui-rule', exact=True)).to_be_visible()
                    page.locator('[data-action="toggle-interceptor"]').click()
                    submit('PUT', '/interceptors')
                    page.locator('[data-action="delete-interceptor"]').click()
                    submit('PUT', '/interceptors')
                    report['checks'].append('independent interceptor add, disable, delete with actual revision CAS')

                    route('settings')
                    page.locator('#f-model_timeout').fill('61')
                    page.locator('#validate-settings').click()
                    expect(page.locator('#settings-validation')).to_be_visible()
                    settings_writes = []
                    def settings_request(request):
                        if '/admin/api/settings' in request.url and request.method in ['POST', 'PATCH']:
                            settings_writes.append(request.method)
                    page.on('request', settings_request)
                    with page.expect_response(lambda response: response.request.method == 'PATCH' and response.url.endswith('/admin/api/settings')) as pending:
                        page.locator('#settings-form button[type="submit"]').click()
                    check(pending.value.ok, f'Settings save failed ({pending.value.status})')
                    expect(page.locator('#main-content')).not_to_have_attribute('aria-busy', 'true')
                    expect(page.locator('#f-model_timeout')).to_have_value('61')
                    page.remove_listener('request', settings_request)
                    check(settings_writes == ['PATCH'], f'Saving settings redundantly validates: {settings_writes}')
                    check(json.loads((temp / 'snapshot.json').read_text(encoding='utf-8'))['preferences']['model_timeout'] == 61, 'Settings were not published to source snapshot')
                    check('重启后恢复源配置' not in page.locator('#main-content').inner_text(), 'Runtime-only warning remains')
                    report['checks'].append('persistent settings publish to source; explicit validation is separate from one-PATCH save')

                    route('plugins')
                    expect(page.get_by_text('尚未安装插件', exact=True)).to_be_visible()
                    page.locator('[data-action="plugin-group"][data-group="builtin"]').click()
                    expect(page.locator('[data-action="plugin-detail"]').first).to_be_visible()
                    page.locator('[data-action="plugin-detail"]').first.click()
                    expect(page.get_by_text('执行版本', exact=True)).to_be_visible()
                    page.locator('#editor-form button[type="submit"]').click()
                    expect(page.locator('#editor-dialog')).not_to_be_visible()
                    page.locator('[data-action="plugin-group"][data-group="installed"]').click()
                    page.locator('[data-action="create-plugin"]').click()
                    expect(page.locator('#plugin-file')).to_have_count(1)
                    expect(page.locator('#editor-form input:not([type="file"]), #editor-form textarea, #editor-form select')).to_have_count(0)
                    page.locator('#plugin-file').set_input_files({'name': 'invalid.json', 'mimeType': 'application/json', 'buffer': b'not-json'})
                    expect(page.locator('#plugin-file-error')).to_be_visible()
                    expect(page.locator('#editor-form button[type="submit"]')).to_be_disabled()
                    package = json.loads(json.dumps(fixture.echo.MANIFEST))
                    package['config_schema'][-1]['required'] = True
                    native = {'format': 'miora.plugin.v1', 'manifest': package, 'endpoint': 'https://not-allowed.invalid', 'service_auth': {'type': 'bearer', 'token': service.token}}
                    def choose_plugin(value):
                        page.locator('#plugin-file').set_input_files({'name': '示例.miora.json', 'mimeType': 'application/json', 'buffer': json.dumps(value).encode()})
                        expect(page.locator('#plugin-file-preview')).to_be_visible()
                        expect(page.locator('#editor-form button[type="submit"]')).to_be_enabled()
                        expect(page.locator('#editor-form input:not([type="file"]), #editor-form textarea, #editor-form select')).to_have_count(0)
                    choose_plugin(native)
                    with page.expect_response(lambda response: response.request.method == 'POST' and response.url.endswith('/admin/api/plugins/upload')) as pending:
                        page.locator('#editor-form button[type="submit"]').click()
                    check(pending.value.status == 400, f'Unexpected upload response: {pending.value.status}: {pending.value.text()}')
                    check('not trusted' in pending.value.json().get('error', {}).get('message', ''), 'Upload failed for a different reason than trust policy')
                    expect(page.locator('.dialog-error')).to_be_visible()
                    native['endpoint'] = f'http://127.0.0.1:{service.server_port}'
                    transfer = page.evaluate_handle('''value => {
                        const transfer = new DataTransfer();
                        transfer.items.add(new File([JSON.stringify(value)], '示例.miora.json', {type: 'application/json'}));
                        return transfer;
                    }''', native)
                    page.locator('#plugin-drop-zone').dispatch_event('drop', {'dataTransfer': transfer})
                    transfer.dispose()
                    expect(page.locator('#plugin-file-preview')).to_be_visible()
                    expect(page.locator('#editor-form button[type="submit"]')).to_be_enabled()
                    for width in [320, 390, 768]:
                        page.set_viewport_size({'width': width, 'height': 844})
                        page.evaluate('() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))')
                        check(page.locator('#editor-dialog').evaluate('modal => modal.scrollWidth <= modal.clientWidth + 1 && modal.getBoundingClientRect().left >= -1 && modal.getBoundingClientRect().right <= innerWidth + 1'), f'Plugin upload at {width}px overflows viewport')
                    shot('plugin-upload-mobile.png')
                    page.set_viewport_size({'width': 1440, 'height': 1000})
                    shot('plugin-upload-desktop.png')
                    submit('POST', '/plugins/upload')
                    expect(page.locator('.plugin-list-item')).to_have_count(1)
                    expect(page.locator('[data-action="edit-plugin"]')).to_have_count(0)
                    shot('plugins-desktop.png')
                    page.locator('[data-action="toggle-plugin"]').click()
                    submit('POST', '/plugins/echo/disable')
                    expect(page.locator('.plugin-list-item .badge')).to_have_text('已停用')
                    page.locator('[data-action="toggle-plugin"]').click()
                    submit('POST', '/plugins/echo/enable')
                    page.locator('[data-action="upload-plugin-update"]').click()
                    wrong_update = dict(native, manifest=dict(native['manifest'], id='different-plugin'))
                    page.locator('#plugin-file').set_input_files({'name': 'wrong.miora.json', 'mimeType': 'application/json', 'buffer': json.dumps(wrong_update).encode()})
                    expect(page.locator('#plugin-file-error')).to_be_visible()
                    expect(page.locator('#editor-form button[type="submit"]')).to_be_disabled()
                    native['manifest']['version'] = '2.0.0'
                    choose_plugin(native)
                    expect(page.locator('#editor-form button[type="submit"]')).to_have_text('安装新版本')
                    submit('POST', '/plugins/upload')
                    expect(page.locator('.plugin-list-version')).to_have_text('v2.0.0')
                    page.locator('[data-action="plugin-detail"]').click()
                    expect(page.locator('#editor-dialog tbody tr')).to_have_count(2)
                    page.keyboard.press('Escape')
                    expect(page.locator('#editor-dialog')).not_to_be_visible()
                    report['checks'].append('native file upload/install/update without configuration even for required channel fields; built-in separation, invalid file, trust rejection, enable/disable and version history')

                    page.set_viewport_size({'width': 390, 'height': 844})
                    route('overview')
                    check(page.evaluate('document.documentElement.scrollWidth <= innerWidth + 1'), 'Narrow overview overflows viewport')
                    shot('overview-mobile.png')
                    page.locator('#menu-toggle').click()
                    expect(page.locator('#menu-toggle')).to_have_attribute('aria-expanded', 'true')
                    shot('navigation-mobile.png')
                    page.keyboard.press('Escape')
                    expect(page.locator('#menu-toggle')).to_have_attribute('aria-expanded', 'false')
                    check(page.evaluate('document.querySelector("#sidebar").inert'), 'Closed mobile navigation remains tabbable')
                    for name in ['channels', 'plugins', 'interceptors', 'keys', 'logs', 'log-settings', 'usage', 'settings']:
                        route(name)
                        check(page.evaluate('document.documentElement.scrollWidth <= innerWidth + 1'), f'Narrow {name} overflows viewport')
                        shot(f'{name}-mobile.png')
                    route('channels')
                    page.locator('[data-action="create-channel"]').click()
                    expect(page.locator('#editor-dialog')).to_be_visible()
                    for tab in ['basic', 'models', 'parameters']:
                        page.locator(f'[data-channel-tab="{tab}"]').click()
                        check_channel_layout()
                        shot(f'channel-{tab}-mobile.png')
                    page.keyboard.press('Escape')
                    expect(page.locator('#editor-dialog')).not_to_be_visible()
                    report['checks'].append('390px login/all nine routes fit viewport, mobile menu/Escape/inert')
                    report['checks'].append('desktop/mobile three-tab channel dialog keeps header, tabs and footer outside scrolling body')

                    page.set_viewport_size({'width': 1440, 'height': 1000})
                    route('overview')
                    page.emulate_media(color_scheme='dark')
                    expect(page.locator('html')).to_have_attribute('data-theme', 'dark')
                    expect(page.get_by_role('combobox', name='外观模式')).to_have_value('system')
                    appearance_requests = []
                    def appearance_request(request):
                        appearance_requests.append(request.url)
                    page.on('request', appearance_request)
                    page.get_by_role('combobox', name='外观模式').select_option('light')
                    expect(page.locator('html')).to_have_attribute('data-theme', 'light')
                    check(not appearance_requests, 'Theme change triggers network requests')
                    page.remove_listener('request', appearance_request)
                    page.reload()
                    expect(page.locator('html')).to_have_attribute('data-theme', 'light')
                    expect(page.get_by_role('combobox', name='外观模式')).to_have_value('light')
                    page.get_by_role('combobox', name='外观模式').select_option('dark')
                    another_page = context.new_page()
                    another_page.goto(origin + '/admin#overview')
                    expect(another_page.locator('html')).to_have_attribute('data-theme', 'dark')
                    another_page.get_by_role('combobox', name='外观模式').select_option('light')
                    expect(page.locator('html')).to_have_attribute('data-theme', 'light')
                    another_page.close()
                    page.get_by_role('combobox', name='外观模式').select_option('system')
                    expect(page.locator('html')).to_have_attribute('data-theme', 'dark')
                    page.emulate_media(color_scheme='light')
                    expect(page.locator('html')).to_have_attribute('data-theme', 'light')
                    check(page.evaluate('localStorage.length === 0 && sessionStorage.length === 0'), 'System mode does not clear the theme override')
                    page.emulate_media(color_scheme=args.color_scheme)
                    report['checks'].append('light/dark/system, automatic OS changes, persistence, cross-tab sync and zero theme-change requests')
                    expect(page.locator('#main-content')).not_to_have_attribute('aria-busy', 'true')
                    page.locator('#logout').click()
                    with page.expect_response(lambda response: response.url.endswith('/admin/api/logout')) as pending:
                        page.locator('#editor-form button[type="submit"]').click()
                    check(pending.value.ok, 'Logout failed')
                    page.wait_for_url('**/login')
                    expect(page.locator('html')).to_have_attribute('data-theme', args.color_scheme)
                    expect(page.get_by_role('combobox', name='外观模式')).to_have_value('system')
                    check(context.request.get(origin + '/admin/api/session').status == 401, 'Logout did not revoke session')
                    report['checks'].append('logout redirects to login and subsequent session request is 401')
                    check(not page_errors, 'Browser JavaScript errors: ' + '\n'.join(page_errors))
                    check(not csp_errors, 'Browser CSP violations: ' + '\n'.join(csp_errors))
                    report['checks'].append('zero browser JavaScript exceptions or CSP violations')

                    # Fault injection is isolated from the real-backend checks.
                    # Assets still come from the candidate gateway; only this
                    # second context's session response is deliberately failed.
                    fault_context = browser.new_context(locale='zh-CN')
                    try:
                        # The document itself remains authenticated; otherwise
                        # the real gateway correctly redirects /admin to login
                        # before the injected session-fetch failure can occur.
                        fault_login = fault_context.request.post(origin + '/admin/api/login',
                            data={'username': 'admin', 'password': password}, headers={'Origin': origin})
                        check(fault_login.ok, f'Fault-context real login failed: {fault_login.status}')
                        fault_page = fault_context.new_page()
                        fault_errors = []
                        document_requests = []
                        fault_page.on('pageerror', lambda error: fault_errors.append(str(error)))
                        fault_page.on('request', lambda request: document_requests.append(request.url)
                                      if request.is_navigation_request() and request.frame == fault_page.main_frame else None)
                        fault_page.route('**/admin/api/session', lambda intercepted: intercepted.fulfill(
                            status=503, content_type='application/json',
                            body=json.dumps({'error': {'code': 'fixture_session_unavailable',
                                                      'message': 'injected session failure'}})))
                        fault_page.goto(origin + '/admin#overview')

                        def failed_session():
                            expect(fault_page.locator('#session-state')).to_have_text('会话验证失败')
                            expect(fault_page.locator('.error-state')).to_be_visible()
                            expect(fault_page.locator('#main-content a[href="/login"]')).to_be_visible()
                            expect(fault_page.locator('[data-action="refresh"]')).to_be_enabled()

                        failed_session()
                        reload_counts = {}
                        for activation in ['pointer', 'keyboard_enter']:
                            before = len(document_requests)
                            refresh = fault_page.locator('[data-action="refresh"]')
                            with fault_page.expect_navigation(wait_until='domcontentloaded'):
                                if activation == 'pointer':
                                    refresh.click()
                                else:
                                    refresh.focus()
                                    refresh.press('Enter')
                            failed_session()
                            fault_page.wait_for_timeout(200)
                            count = len(document_requests) - before
                            check(count == 1, f'{activation}: expected one reload, got {count}')
                            reload_counts[activation] = count
                        check(not fault_errors, 'Fault-page JavaScript exceptions: ' + '\n'.join(fault_errors))
                        report['fault_injection'] = {
                            'session_http_status': 503, 'scope': 'separate browser context only',
                            'assets': 'actual embedded candidate assets', 'reload_document_requests': reload_counts,
                            'error_and_login_link_preserved': True, 'javascript_exceptions': fault_errors,
                        }
                        report['checks'].append('injected session failure: pointer and keyboard each reload once; error/login entry retained')
                    finally:
                        fault_context.close()
                    restricted_context = browser.new_context(color_scheme='dark')
                    restricted_page = restricted_context.new_page()
                    restricted_errors = []
                    restricted_page.on('pageerror', lambda error: restricted_errors.append(str(error)))
                    restricted_page.add_init_script("Object.defineProperty(window, 'localStorage', { get() { throw new DOMException('Blocked', 'SecurityError'); } });")
                    try:
                        restricted_page.goto(origin + '/login')
                        expect(restricted_page.locator('html')).to_have_attribute('data-theme', 'dark')
                        restricted_page.get_by_role('combobox', name='外观模式').select_option('light')
                        expect(restricted_page.locator('html')).to_have_attribute('data-theme', 'light')
                        check(not restricted_errors, 'Blocked storage breaks the login page')
                        report['checks'].append('blocked localStorage preserves system theme, manual switching and login rendering')
                    finally:
                        restricted_context.close()
                    report['passed'] = True
                    browser.close()
            except Exception as error:
                report['passed'] = False
                report['failure'] = str(error)
                raise
            finally:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
                service.shutdown()
                service.server_close()
                (artifacts / 'browser-report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
    print('PASS: real browser regression, ' + str(len(report['checks'])) + ' checks; screenshots: ' + str(artifacts))


if __name__ == '__main__':
    main()
