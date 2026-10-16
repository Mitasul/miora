"""Real gateway usage, configured-price snapshots and protocol-mode regression."""
import argparse
from contextlib import closing
import http.client
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import secrets
import socket
import sqlite3
import subprocess
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parents[2]


class Upstream(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        payload = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        self.server.hits.append((self.path, payload))
        variant = payload.get('variant')
        if variant == 'failure' or payload.get('input') == 'fail' or any(message.get('content') == 'fail' for message in payload.get('messages', [])):
            self.send_response(500)
            self.end_headers()
            self.wfile.write(b'{"error":{"message":"fixture failure"}}')
            return
        usage = {'input_tokens': 100, 'output_tokens': 20, 'input_tokens_details': {'cached_tokens': 60}}
        if variant == 'unknown':
            usage = {'input_tokens': 30, 'output_tokens': 5}
        if variant == 'zero':
            usage = {'input_tokens': 0, 'output_tokens': 0, 'input_tokens_details': {'cached_tokens': 0}}
        response = {'id': 'resp_fixture', 'object': 'response', 'status': 'completed', 'model': payload['model'], 'output': [{'type': 'message', 'role': 'assistant', 'content': [{'type': 'output_text', 'text': 'fixture output'}]}], 'usage': usage, 'native_marker': True}
        if self.path.endswith('/chat/completions'):
            response = {'id': 'chat_fixture', 'object': 'chat.completion', 'model': payload['model'], 'choices': [{'index': 0, 'message': {'role': 'assistant', 'content': 'fixture output'}, 'finish_reason': 'stop'}], 'usage': {'prompt_tokens': usage['input_tokens'], 'completion_tokens': usage['output_tokens'], 'prompt_tokens_details': usage.get('input_tokens_details', {})}}
        self.send_response(200)
        self.send_header('Content-Type', 'text/event-stream' if payload.get('stream') else 'application/json')
        self.end_headers()
        if payload.get('stream'):
            for event in [{'type': 'response.created', 'response': {'id': 'resp_fixture', 'object': 'response', 'status': 'in_progress'}}, {'type': 'response.output_text.delta', 'delta': 'fixture output'}, {'type': 'response.completed', 'response': response}]:
                self.wfile.write(('event: ' + event['type'] + '\ndata: ' + json.dumps(event) + '\n\n').encode())
                self.wfile.flush()
        else:
            self.wfile.write(json.dumps(response).encode())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', default='target/debug/uni-api-front.exe')
    parser.add_argument('--browser', action='store_true')
    parser.add_argument('--artifacts', default='.artifacts/model-usage')
    args = parser.parse_args()
    binary = (ROOT / args.binary).resolve()
    artifacts = (ROOT / args.artifacts).resolve()
    artifacts.mkdir(parents=True, exist_ok=True)
    service = ThreadingHTTPServer(('127.0.0.1', 0), Upstream)
    service.hits = []
    threading.Thread(target=service.serve_forever, daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix='miora-usage-') as directory:
            root = Path(directory)
            with socket.socket() as sock:
                sock.bind(('127.0.0.1', 0))
                port = sock.getsockname()[1]
            origin = f'http://127.0.0.1:{port}'
            password = 'UsageFixture-' + secrets.token_hex(16)
            config = {'providers': [{'provider': 'usage-fixture', 'engine': 'gpt', 'base_url': f'http://127.0.0.1:{service.server_port}/v1', 'api': 'private-upstream-key', 'model': [{'upstream-m': 'public-m'}], 'preferences': {'protocol_mode': 'native', 'AUTO_RETRY': False, 'cooldown_period': 0, 'usage_pricing': {'input': 2, 'output': 8, 'cache_read': .2}}}], 'api_keys': [{'api': 'private-caller-key', 'model': ['all'], 'role': 'user'}]}
            config['providers'][0]['preferences']['usage_pricing']['models'] = {f'other-{index}': {'input': 3, 'output': 9} for index in range(499)}
            config['providers'][0]['preferences']['usage_pricing']['models']['public-m'] = {'input': 2, 'output': 8, 'cache_read': .2}
            source = root / 'api.json'
            source.write_text(json.dumps(config), encoding='utf-8')
            env = {key: value for key, value in os.environ.items() if not key.startswith(('MIORA_ADMIN_', 'MIORA_PLUGIN_', 'MIORA_REQUEST_LOG_', 'FACTS_S3_')) and key != 'CONFIG_URL'}
            env.update(PORT=str(port), DISABLE_DATABASE='false', DB_TYPE='sqlite', DB_PATH=str(root / 'stats.db'), UNI_API_CONFIG_PATH=str(source), RUST_RESPONSES_CONFIG_SNAPSHOT_PATH=str(root / 'snapshot.json'), UNI_API_SHARED_MEMORY_RESERVATION_PATH=str(root / 'ledger'), RUST_REQUEST_SPOOL_DIRECTORY=str(root / 'spool'), RUST_REQUEST_SPOOL_DISK_RESERVE_BPS='0', RUST_REQUEST_SPOOL_INODE_RESERVE_BPS='0', MIORA_ADMIN_USERNAME='admin', MIORA_ADMIN_PASSWORD=password, MIORA_ADMIN_SECURE_COOKIE='false', MIORA_ADMIN_ORIGIN=origin, MIORA_PLUGIN_STORE_DIR=str(root / 'plugins'), MIORA_REQUEST_LOG_DIR=str(root / 'logs'), NO_PROXY='127.0.0.1,localhost')
            cookie, csrf, process = '', '', None

            def request(method, path, body=None, admin=False, api_key='private-caller-key'):
                connection = http.client.HTTPConnection('127.0.0.1', port, timeout=30)
                headers = {'Content-Type': 'application/json', 'Origin': origin}
                if admin:
                    headers.update(Cookie=cookie, **{'X-CSRF-Token': csrf})
                elif path.startswith('/v1/'):
                    headers['Authorization'] = 'Bearer ' + api_key
                connection.request(method, path, None if body is None else json.dumps(body).encode(), headers)
                response = connection.getresponse()
                raw = response.read()
                result = response.status, dict((key.lower(), value) for key, value in response.getheaders()), raw
                connection.close()
                return result

            def admin(method, path, body=None):
                status, _, raw = request(method, '/admin/api/' + path, body, True)
                assert status == 200, (status, path, raw[:500])
                return json.loads(raw)

            def usage(count, filters='model=public-m'):
                for _ in range(100):
                    data = admin('GET', 'usage?days=1&' + filters)
                    if data['summary']['request_count'] == count:
                        return data
                    time.sleep(.1)
                raise AssertionError(('missing persisted usage', count, data))

            with (root / 'gateway.log').open('w+', encoding='utf-8') as log:
                def start():
                    nonlocal process, cookie, csrf
                    process = subprocess.Popen([str(binary)], cwd=root, env=env, stdout=log, stderr=log, creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
                    for _ in range(200):
                        if process.poll() is not None:
                            raise AssertionError('Gateway exited during startup')
                        try:
                            if request('GET', '/healthz')[0] == 200:
                                break
                        except OSError:
                            pass
                        time.sleep(.1)
                    status, headers, raw = request('POST', '/admin/api/login', {'username': 'admin', 'password': password})
                    assert status == 200, raw
                    cookie = headers['set-cookie'].split(';')[0]
                    csrf = json.loads(raw)['csrf_token']

                def stop():
                    if process and process.poll() is None:
                        process.terminate()
                        process.wait(10)

                try:
                    start()
                    for variant in [None, 'stream', 'unknown', 'zero']:
                        payload = {'model': 'public-m', 'input': 'fixture input'}
                        if variant == 'stream':
                            payload['stream'] = True
                        elif variant:
                            payload['variant'] = variant
                        status, _, raw = request('POST', '/v1/responses', payload)
                        assert status == 200, raw
                        if variant is None:
                            assert json.loads(raw)['native_marker'] is True
                        elif variant == 'stream':
                            assert b'event: response.completed' in raw and b'"native_marker": true' in raw, raw
                        assert service.hits[-1][0] == '/v1/responses'
                        assert 'input' in service.hits[-1][1] and 'messages' not in service.hits[-1][1]
                    revision = admin('GET', 'channels')['revision']
                    admin('PATCH', 'channels', {'revision': revision, 'provider': 'usage-fixture', 'patch': {'preferences': {'protocol_mode': 'auto'}}})
                    assert request('POST', '/v1/responses', {'model': 'public-m', 'input': 'convert me'})[0] == 200
                    assert service.hits[-1][0] == '/v1/chat/completions' and 'messages' in service.hits[-1][1]
                    assert request('POST', '/v1/responses', {'model': 'public-m', 'input': 'fail', 'variant': 'failure'})[0] >= 400
                    data = usage(6)
                    summary = data['summary']
                    for key, expected in {'input_tokens': 330, 'output_tokens': 65, 'total_tokens': 395, 'cache_read_tokens': 180, 'cache_miss_tokens': 120, 'cache_hit_rate': .6, 'failure_count': 1, 'priced_requests': 4}.items():
                        assert summary[key] == expected, (key, summary)
                    assert abs(summary['estimated_cost'] - .000756) < 1e-12, summary
                    assert summary['cache_write_tokens'] is None
                    assert summary['channel_count'] == 1
                    assert data['channels'][0]['channel'] == 'usage-fixture'
                    assert data['channels'][0]['request_count'] == summary['request_count']
                    assert data['channels'][0]['failure_count'] == 1
                    assert len(data['channels']) == 1
                    assert sum(row['request_count'] for row in data['trend']) == summary['request_count']
                    assert sum(row['failure_count'] for row in data['trend']) == summary['failure_count']
                    assert abs(sum(row['estimated_cost'] or 0 for row in data['trend']) - summary['estimated_cost']) < 1e-12
                    assert data['bucket_seconds'] == 3600
                    with closing(sqlite3.connect(root / 'stats.db')) as database:
                        stored = json.loads(database.execute('SELECT usage_details FROM request_stats WHERE usage_details IS NOT NULL ORDER BY id LIMIT 1').fetchone()[0])
                    assert stored['pricing'] == {'input': 2, 'output': 8, 'cache_read': .2}
                    assert len(json.dumps(stored['pricing'])) < 100
                    assert 'private-' not in json.dumps(data)
                    revision = admin('GET', 'channels')['revision']
                    prices = config['providers'][0]['preferences']['usage_pricing']
                    prices['models']['public-m'] = {'input': 10, 'output': 20, 'cache_read': 1}
                    admin('PATCH', 'channels', {'revision': revision, 'provider': 'usage-fixture', 'patch': {'preferences': {'protocol_mode': 'native', 'usage_pricing': prices}}})
                    assert request('POST', '/v1/responses', {'model': 'public-m', 'input': 'new prices'})[0] == 200
                    summary = usage(7)['summary']
                    assert abs(summary['estimated_cost'] - .001616) < 1e-12, summary
                    assert admin('GET', 'usage?days=1&channel=missing')['summary']['request_count'] == 0
                    with closing(sqlite3.connect(root / 'stats.db')) as database:
                        database.execute("INSERT INTO request_stats(model,provider,prompt_tokens,completion_tokens) VALUES('legacy-model','legacy',42,8)")
                        database.commit()
                    time.sleep(1.1)
                    legacy = admin('GET', 'usage?days=1&model=legacy-model')['summary']
                    assert legacy['cache_read_tokens'] is None and legacy['estimated_cost'] is None
                    assert legacy['total_prompt_tokens'] == 42
                    print('PASS actual JSON/SSE usage, cache-inclusive accounting, unknown vs zero, failed requests, configured prices, immutable price snapshots, channel/model filters and legacy history')
                    if args.browser:
                        from playwright.sync_api import sync_playwright, expect
                        with sync_playwright() as playwright:
                            browser = playwright.chromium.launch(executable_path=r'C:\Program Files\Google\Chrome\Application\chrome.exe', headless=True)
                            page = browser.new_page(viewport={'width': 1440, 'height': 1000}, locale='zh-CN')
                            errors = []
                            page.on('pageerror', lambda error: errors.append(str(error)))
                            page.goto(origin + '/login')
                            page.locator('#password').fill(password)
                            page.locator('#login-submit').click()
                            page.wait_for_url('**/admin#overview')
                            expect(page.locator('.usage-summary-item')).to_have_count(8)
                            expect(page.locator('.model-usage-table tbody tr')).to_have_count(2)
                            expect(page.locator('.usage-trend-chart')).to_be_visible()
                            usage_requests = []
                            page.on('request', lambda request: usage_requests.append(request.url) if '/admin/api/usage?' in request.url else None)
                            page.locator('#f-trend_metric').select_option('estimated_cost')
                            expect(page.locator('.usage-trend-chart')).to_have_attribute('aria-label', '费用估算 · USD趋势，点击柱形查看该时段日志')
                            page.locator('#f-group').select_option('channel')
                            expect(page.get_by_role('heading', name='渠道对比')).to_be_visible()
                            page.locator('.model-usage-table tbody tr').filter(has_text='usage-fixture').locator('[data-action="usage-detail"]').click()
                            expect(page.locator('#dialog-title')).to_have_text('usage-fixture')
                            page.keyboard.press('Escape')
                            assert not usage_requests, usage_requests
                            page.locator('.model-usage-name', has_text='usage-fixture').click()
                            expect(page.locator('#f-channel')).to_have_value('usage-fixture')
                            assert page.locator('#f-from').input_value() and page.locator('#f-to').input_value()
                            page.goto(origin + '/admin#overview')
                            expect(page.locator('.model-usage-table tbody tr')).to_have_count(2)
                            page.locator('#f-group').select_option('model')
                            trend_link = page.locator('.usage-trend-chart a').first.get_attribute('href')
                            page.locator('.usage-trend-chart a').first.click()
                            expect(page.locator('#request-log-filter')).to_be_visible()
                            assert page.url.endswith(trend_link), page.url
                            page.goto(origin + '/admin#overview')
                            expect(page.locator('.model-usage-table tbody tr')).to_have_count(2)
                            page.locator('#f-group').select_option('model')
                            page.screenshot(path=str(artifacts / 'overview-model-usage.png'), full_page=True)
                            page.get_by_role('combobox', name='外观模式').select_option('dark')
                            expect(page.locator('html')).to_have_attribute('data-theme', 'dark')
                            page.evaluate('() => Promise.all(document.getAnimations().filter(animation => animation.effect.getTiming().iterations !== Infinity).map(animation => animation.finished.catch(() => {})))')
                            page.screenshot(path=str(artifacts / 'overview-model-usage-dark.png'), full_page=True)
                            page.locator('#usage-filter input[name="model"]').fill('public-m')
                            page.locator('#usage-filter button[type="submit"]').click()
                            expect(page.locator('.model-usage-table tbody tr')).to_have_count(1)
                            page.locator('[data-action="usage-detail"]').click()
                            expect(page.locator('#dialog-title')).to_have_text('public-m')
                            expect(page.get_by_text('其中 1 小时写入', exact=True)).to_be_visible()
                            page.keyboard.press('Escape')
                            for width in [320, 390, 768, 1440]:
                                page.set_viewport_size({'width': width, 'height': 1000})
                                page.evaluate('() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))')
                                page.evaluate('() => Promise.all(document.querySelector("#sidebar").getAnimations().map(animation => animation.finished.catch(() => {})))')
                                assert page.evaluate('document.documentElement.scrollWidth <= innerWidth + 1'), width
                                if width <= 760:
                                    assert page.locator('#sidebar').evaluate('(element) => element.getBoundingClientRect().right <= 1')
                                if width == 390:
                                    page.screenshot(path=str(artifacts / 'overview-model-usage-dark-mobile.png'), full_page=True)
                            page.locator('.model-usage-name').click()
                            expect(page.locator('#f-model')).to_have_value('public-m')
                            page.goto(origin + '/admin#channels')
                            page.locator('[data-action="edit-channel"]').click()
                            expect(page.locator('#f-protocol_mode')).to_have_value('native')
                            page.locator('[data-channel-tab="models"]').click()
                            page.locator('[data-price-row="0"] summary').click()
                            expect(page.locator('[data-price-index="0"][data-price-key="input"]')).to_have_value('10')
                            page.screenshot(path=str(artifacts / 'channel-model-pricing.png'), full_page=True)
                            page.locator('[data-price-index="0"][data-price-key="cache_read"]').fill('')
                            page.locator('#editor-form button[type="submit"]').click()
                            expect(page.locator('#dialog-title')).not_to_be_visible()
                            detail = admin('GET', 'channels/detail?provider=usage-fixture')
                            assert detail['effective']['preferences']['usage_pricing']['models']['public-m']['cache_read'] is None
                            assert detail['effective']['preferences']['usage_pricing']['cache_read'] == .2
                            assert not errors, errors
                            browser.close()
                        print('PASS real browser: model overview, detail, filtering, request-log drilldown, 320–1440px layout, protocol mode and pricing controls')
                    stop()
                    start()
                    assert abs(usage(7)['summary']['estimated_cost'] - .001616) < 1e-12
                    print('PASS persisted metrics and historical price snapshots survive gateway restart')
                    detail = admin('GET', 'channels/detail?provider=usage-fixture')['effective']
                    catalog = detail['preferences']['usage_pricing']['models']
                    catalog.pop('other-0')
                    catalog['public-m'] = {'input': 10, 'output': 20, 'cache_read': 1}
                    catalog['budget-model'] = {'input': 4, 'output': 16, 'cache_read': .4}
                    admin('PATCH', 'channels', {'revision': admin('GET', 'channels')['revision'], 'provider': 'usage-fixture', 'patch': {
                        'model': [{'upstream-m': 'public-m'}, {'upstream-m': 'budget-model'}],
                        'preferences': {'usage_pricing': {'models': catalog}}
                    }})
                    admin('POST', 'channels', {'revision': admin('GET', 'channels')['revision'], 'channel': {
                        'provider': 'second-priced', 'engine': 'gpt', 'base_url': f'http://127.0.0.1:{service.server_port}/v1',
                        'api': 'private-second-upstream', 'model': [{'upstream-m': 'public-m'}],
                        'preferences': {'protocol_mode': 'native', 'cooldown_period': 0, 'api_key_cooldown_period': 0, 'usage_pricing': {'models': {'public-m': {'input': 30, 'output': 60, 'cache_read': 3}}}}
                    }})
                    first_key = admin('POST', 'api-keys', {'revision': admin('GET', 'api-keys')['revision'], 'models': ['usage-fixture/*']})['api_key']
                    second_key = admin('POST', 'api-keys', {'revision': admin('GET', 'api-keys')['revision'], 'models': ['second-priced/*']})['api_key']
                    for model, key in [('public-m', first_key), ('public-m', second_key), ('budget-model', first_key)]:
                        assert request('POST', '/v1/responses', {'model': model, 'input': 'model-specific prices'}, api_key=key)[0] == 200
                    for filters, count, cost in [('model=public-m&channel=usage-fixture', 8, .002476), ('model=public-m&channel=second-priced', 1, .00258), ('model=budget-model&channel=usage-fixture', 1, .000504)]:
                        assert abs(usage(count, filters)['summary']['estimated_cost'] - cost) < 1e-12
                    combined = usage(9)
                    assert {row['channel']: row['request_count'] for row in combined['channels']} == {'usage-fixture': 8, 'second-priced': 1}
                    assert abs(sum(row['estimated_cost'] for row in combined['channels']) - combined['summary']['estimated_cost']) < 1e-12
                    daily = admin('GET', 'usage?days=7&model=public-m')
                    assert daily['bucket_seconds'] == 86400 and sum(row['request_count'] for row in daily['trend']) == 9
                    with closing(sqlite3.connect(root / 'stats.db')) as database:
                        snapshots = {(model, provider): json.loads(details)['pricing'] for model, provider, details in database.execute('SELECT model,provider,usage_details FROM request_stats ORDER BY id DESC LIMIT 3')}
                    assert snapshots[('public-m', 'usage-fixture')] == {'input': 10, 'output': 20, 'cache_read': 1}
                    assert snapshots[('public-m', 'second-priced')] == {'input': 30, 'output': 60, 'cache_read': 3}
                    assert snapshots[('budget-model', 'usage-fixture')] == {'input': 4, 'output': 16, 'cache_read': .4}
                    stop()
                    start()
                    assert request('POST', '/v1/responses', {'model': 'budget-model', 'input': 'prices after restart'}, api_key=first_key)[0] == 200
                    assert abs(usage(2, 'model=budget-model&channel=usage-fixture')['summary']['estimated_cost'] - .001008) < 1e-12
                    print('PASS per-model prices, public aliases, same-model channel isolation, immutable snapshots and configured prices after restart')
                    failed_requests = 0
                    for hedging in (False, True):
                        stop()
                        saved = json.loads((root / 'snapshot.json').read_text(encoding='utf-8'))['api_config']
                        saved.setdefault('preferences', {})['hedging'] = {'enabled': hedging, 'max_inflight_attempts': 2}
                        source.write_text(json.dumps(saved), encoding='utf-8')
                        start()
                        for endpoint in ('/v1/chat/completions', '/v1/responses'):
                            hits_before = len(service.hits)
                            payload = {'model': 'public-m', 'messages': [{'role': 'user', 'content': 'fail'}]} if endpoint.endswith('/chat/completions') else {'model': 'public-m', 'input': 'fail'}
                            assert request('POST', endpoint, payload, api_key=second_key)[0] >= 400
                            failed_requests += 1
                            failed = usage(1 + failed_requests, 'model=public-m&channel=second-priced')
                            assert failed['summary']['failure_count'] == failed_requests, failed
                            assert len(failed['channels']) == 1 and failed['channels'][0]['channel'] == 'second-priced'
                            assert sum(row['request_count'] for row in failed['trend']) == 1 + failed_requests
                            assert len(service.hits) - hits_before > 1, (hedging, endpoint, service.hits[hits_before:])
                            with closing(sqlite3.connect(root / 'stats.db')) as database:
                                provider, model, usage_details, timing_spans = database.execute('SELECT provider,model,usage_details,timing_spans FROM request_stats ORDER BY id DESC LIMIT 1').fetchone()
                            assert provider == 'second-priced' and model == 'public-m'
                            assert json.loads(usage_details)['estimated_cost'] is None
                            if endpoint.endswith('/chat/completions'):
                                assert json.loads(timing_spans)['terminal'] == ('hedge_exhausted' if hedging else 'route_exhausted')
                    print('PASS exhausted sequential/hedged/native requests keep the actual channel and count once despite physical retries; unknown failed usage is not priced as free')
                except Exception:
                    log.flush()
                    print((root / 'gateway.log').read_text(encoding='utf-8', errors='replace')[-7000:])
                    raise
                finally:
                    stop()
    finally:
        service.shutdown()
        service.server_close()


if __name__ == '__main__':
    main()
