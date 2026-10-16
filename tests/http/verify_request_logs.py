#!/usr/bin/env python3
"""Focused request-log E2E: real gateway, local upstream, separate statistics DB.

No production configuration, vendor endpoint, checksum or full-suite runner is used.
"""
import copy
from contextlib import closing
import http.server
import json
import os
from pathlib import Path
import secrets
import shutil
import socket
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse

from http_fixture_process import wait_for_gateway, stop_gateway
from verify_admin import Client

ALL_FIELDS = ['request_headers', 'request_body', 'upstream_request_headers', 'upstream_request_body',
              'upstream_response_headers', 'upstream_response_body', 'response_headers', 'response_body']
SVG = '<svg xmlns="http://www.w3.org/2000/svg"><text>log markup must stay text</text></svg>'


class Upstream(http.server.BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'

    def log_message(self, *_):
        pass

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        self.server.hits.append(body)
        if body.get('stream'):
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.send_header('Transfer-Encoding', 'chunked')
            self.send_header('Connection', 'close')
            self.end_headers()
            for _ in range(96):
                event = {'id': 'chunk-log', 'object': 'chat.completion.chunk', 'model': 'vendor-response-model',
                         'choices': [{'index': 0, 'delta': {'content': 'streamed text ' * 160}, 'finish_reason': None}]}
                self.chunk(b'data: ' + json.dumps(event).encode() + b'\n\n')
            self.chunk(b'data: [DONE]\n\n')
            self.wfile.write(b'0\r\n\r\n')
            self.wfile.flush()
            self.close_connection = True
            return
        tool = any(message.get('content') == 'tool-only' for message in body.get('messages', []))
        message = {'role': 'assistant', 'content': SVG}
        if tool:
            message = {'role': 'assistant', 'content': None, 'tool_calls': [
                {'id': 'call_log', 'type': 'function', 'function': {'name': 'fixture_tool', 'arguments': '{}'}}]}
        # Usage intentionally absent: no usage must not be mistaken for failure.
        value = {'id': 'chat-log', 'object': 'chat.completion', 'model': 'vendor-response-model',
                 'choices': [{'index': 0, 'message': message, 'finish_reason': 'tool_calls' if tool else 'stop'}]}
        raw = json.dumps(value).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('X-Api-Key', 'response-header-secret')
        self.send_header('Content-Length', str(len(raw)))
        self.send_header('Connection', 'close')
        self.end_headers()
        self.wfile.write(raw)
        self.wfile.flush()
        self.close_connection = True

    def chunk(self, data):
        self.wfile.write(f'{len(data):x}\r\n'.encode() + data + b'\r\n')
        self.wfile.flush()


def free_port():
    with socket.socket() as connection:
        connection.bind(('127.0.0.1', 0))
        return connection.getsockname()[1]


def verify(binary):
    upstream = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Upstream)
    upstream.hits = []
    threading.Thread(target=upstream.serve_forever, daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix='miora-request-logs-') as directory:
            root = Path(directory)
            executable = root / ('gateway.exe' if os.name == 'nt' else 'gateway')
            shutil.copy2(binary, executable)
            config = {'providers': [{'provider': 'fixture', 'engine': 'gpt',
                'base_url': f'http://127.0.0.1:{upstream.server_port}/v1/chat/completions',
                'api': 'provider-secret', 'model': [{'upstream-model': 'public-model'}],
                'preferences': {'parameter_policy': {'headers': [
                    {'name': 'X-Private-Note', 'mode': 'set', 'value': 'marked-header-secret', 'secret': True, 'enabled': True}]}}}],
                'api_keys': [{'api': 'caller-secret', 'role': 'user', 'model': ['all'], 'preferences': {'AUTO_RETRY': False}}]}
            (root / 'api.json').write_text(json.dumps(config), encoding='utf-8')
            port = free_port()
            origin = f'http://127.0.0.1:{port}'
            password = secrets.token_urlsafe(24)
            env = {key: value for key, value in os.environ.items()
                   if not key.startswith(('MIORA_', 'FACTS_S3_', 'UNI_API_CONTROL_RESTORE')) and key != 'CONFIG_URL'}
            env.update(PORT=str(port), DISABLE_DATABASE='false', DB_TYPE='sqlite', DB_PATH=str(root / 'stats.db'),
                       MIORA_ADMIN_USERNAME='admin', MIORA_ADMIN_PASSWORD=password,
                       MIORA_ADMIN_SECURE_COOKIE='false', MIORA_ADMIN_ORIGIN=origin,
                       UNI_API_CONFIG_PATH=str(root / 'api.json'),
                       RUST_RESPONSES_CONFIG_SNAPSHOT_PATH=str(root / 'snapshot.json'),
                       UNI_API_SHARED_MEMORY_RESERVATION_PATH=str(root / 'ledger'),
                       RUST_REQUEST_SPOOL_DIRECTORY=str(root / 'spool'),
                       MIORA_PLUGIN_STORE_DIR=str(root / 'plugins'),
                       MIORA_REQUEST_LOG_DIR=str(root / 'request-logs'), NO_PROXY='127.0.0.1,localhost')
            process = None
            with (root / 'runtime.log').open('wb') as log:
                def start():
                    child = subprocess.Popen([str(executable)], cwd=root, env=env, stdout=log, stderr=log, creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
                    try:
                        wait_for_gateway(child, port)
                    except Exception:
                        stop_gateway(child)
                        raise
                    return child

                def login():
                    client = Client(origin)
                    status, headers, session, _ = client.request('POST', '/admin/api/login', {'username': 'admin', 'password': password})
                    assert status == 200, (status, session)
                    client.cookie = headers['set-cookie'].split(';', 1)[0]
                    client.csrf = session['csrf_token']
                    return client

                process = start()
                client = login()

                def api(method, path, body=None, expected=200):
                    result = client.request(method, '/admin/api' + path, body, headers={'X-CSRF-Token': client.csrf})
                    assert result[0] == expected, (method, path, result[0], result[2])
                    return result

                def set_settings(update):
                    snapshot = api('GET', '/log-settings')[2]
                    settings = copy.deepcopy(snapshot['settings'])
                    update(settings)
                    return api('PUT', '/log-settings', {'revision': snapshot['revision'], 'settings': settings})[2]

                def detail(record_id):
                    return api('GET', '/request-logs/' + record_id)[2]

                def wait_record(request_id):
                    deadline = time.monotonic() + 10
                    while time.monotonic() < deadline:
                        rows = api('GET', '/request-logs?' + urllib.parse.urlencode({'request_id': request_id}))[2]['rows']
                        if rows and not rows[0]['active']:
                            return detail(rows[0]['id'])
                        time.sleep(.05)
                    raise AssertionError('request log did not finish for ' + request_id)

                def send(request_id, *, stream=False, tool=False):
                    body = {'model': 'public-model', 'messages': [{'role': 'user', 'content': 'tool-only' if tool else 'fixture request'}],
                            'metadata': {'api_key': 'body-secret'}, 'stream': stream}
                    status, _, data, raw = client.request('POST', '/v1/chat/completions', body,
                        headers={'Authorization': 'Bearer caller-secret', 'X-Request-Id': request_id,
                                 'Cookie': 'private-cookie=fixture-cookie-secret'}, authenticated=False)
                    assert status == 200, (status, data)
                    if stream:
                        assert b'[DONE]' in raw and len(raw) > 100000
                    return wait_record(request_id)

                def fields(document, name):
                    return [field for field in document['fields'] if field['field'] == name]

                def field_path(document, field, operation='content'):
                    query = {'field': field['field']}
                    if field['attempt'] is not None:
                        query['attempt'] = field['attempt']
                    return '/request-logs/' + document['record']['id'] + '/' + operation + '?' + urllib.parse.urlencode(query)

                def stats_count():
                    with closing(sqlite3.connect(root / 'stats.db')) as connection:
                        return connection.execute('SELECT COUNT(*) FROM request_stats').fetchone()[0]

                def wait_stats():
                    expected = len(upstream.hits)
                    deadline = time.monotonic() + 5
                    while time.monotonic() < deadline:
                        count = stats_count()
                        if count == expected:
                            return count
                        time.sleep(.05)
                    raise AssertionError(('statistics writer did not settle', stats_count(), expected))

                try:
                    snapshot = api('GET', '/log-settings')[2]
                    assert snapshot['backend_enabled'] is True
                    assert snapshot['settings']['body_retention_hours'] == 0
                    api('GET', '/request-logs', expected=200)
                    unauth = Client(origin)
                    assert unauth.request('GET', '/admin/api/request-logs', authenticated=False)[0] == 401
                    settings_update = {'revision': snapshot['revision'], 'settings': snapshot['settings']}
                    assert client.request('PUT', '/admin/api/log-settings', settings_update)[0] == 403
                    assert client.request('POST', '/admin/api/request-logs/cleanup', {
                        'dry_run': True, 'action': 'delete_rows', 'filters': {}})[0] == 403
                    assert client.request('POST', '/admin/api/request-logs/cleanup', {
                        'dry_run': True, 'action': 'delete_rows', 'filters': {}},
                        headers={'X-CSRF-Token': 'not-the-session-csrf'})[0] == 403
                    assert api('GET', '/log-settings')[2]['revision'] == snapshot['revision']
                    print('PASS access: account session and CSRF are required for log settings and cleanup')
                    off = send('1' * 32)
                    assert off['record']['status'] == 200 and not any(field['available'] for field in off['fields'])
                    assert not any(path.is_file() for path in (root / 'request-logs/bodies').rglob('*'))
                    print('PASS default: metadata exists, zero-hour policy stores no raw content')

                    def enable(settings):
                        settings['enabled'] = True
                        settings['capture_fields'] = ALL_FIELDS
                        settings['body_retention_hours'] = 24
                        settings['row_retention'] = {'mode': 'days', 'days': 7}
                        settings['limits'] = {'per_request_bytes': 1048576, 'total_body_bytes': 8 * 1048576}
                    saved_settings = set_settings(enable)
                    captured = send('2' * 32)
                    record = captured['record']
                    assert record['channel'] == 'fixture'
                    assert record['requested_model'] == 'public-model' and record['sent_model'] == 'upstream-model'
                    assert record['response_model'] == 'vendor-response-model', record
                    assert record['status'] == 200 and record['relay_complete'] is True
                    combined = json.dumps(captured)
                    for name in ALL_FIELDS:
                        captured_fields = fields(captured, name)
                        assert captured_fields and any(field['available'] for field in captured_fields), name
                        for field in captured_fields:
                            if not field['available']:
                                continue
                            content = api('GET', field_path(captured, field))[2]
                            assert field['observed_bytes'] > 0 and field['saved_bytes'] > 0
                            combined += content['content']
                    for secret in ('caller-secret', 'provider-secret', 'body-secret', 'fixture-cookie-secret',
                                   'marked-header-secret', 'response-header-secret'):
                        assert secret not in combined, 'raw secret found in logs: ' + secret
                    response_field = next(field for field in fields(captured, 'response_body') if field['available'])
                    download = api('GET', field_path(captured, response_field, 'download'))
                    assert 'attachment' in download[1].get('content-disposition', '')
                    assert download[1].get('content-type', '').startswith('application/octet-stream')
                    assert b'<svg' in download[3]
                    tool = send('3' * 32, tool=True)
                    assert tool['record']['status'] == 200 and tool['record']['relay_complete'] is True
                    print('PASS capture: all four stages, three model names, default secrets, SVG text download, no-usage/tool-only success')

                    def small(settings):
                        settings['capture_fields'] = ['upstream_response_body', 'response_body']
                        settings['limits'] = {'per_request_bytes': 2048, 'total_body_bytes': 4096}
                    set_settings(small)
                    long_stream = send('4' * 32, stream=True)
                    assert long_stream['record']['status'] == 200 and long_stream['record']['relay_complete'] is True
                    blobs = fields(long_stream, 'upstream_response_body') + fields(long_stream, 'response_body')
                    assert blobs and any(field['observed_bytes'] > field['saved_bytes'] and not field['complete'] for field in blobs)
                    assert sum(field['saved_bytes'] for field in long_stream['fields']) <= 2048
                    for index in range(5, 13):
                        latest = send(f'{index:032x}')
                    available_bytes = 0
                    evicted = False
                    for row in api('GET', '/request-logs?page_size=100')[2]['rows']:
                        row_detail = detail(row['id'])
                        available_bytes += sum(field['saved_bytes'] for field in row_detail['fields'] if field['available'])
                        evicted |= any(field['availability'] == 'evicted' for field in row_detail['fields'])
                    assert available_bytes <= 4096 and evicted, (available_bytes, evicted)
                    print('PASS budgets: long stream still completes; captured prefix is marked; old raw content is evicted without deleting rows')

                    target_id = latest['record']['id']
                    target_field = next(field for field in fields(latest, 'response_body') if field['available'])
                    plan = api('POST', '/request-logs/cleanup', {
                        'dry_run': True, 'action': 'clear_fields', 'fields': ['response_body'],
                        'filters': {'channel': 'fixture'}})[2]
                    assert plan['matched_rows'] > 0 and plan['estimated_bytes'] > 0
                    api('GET', field_path(latest, target_field))
                    after_preview = send('f' * 32)
                    before_stats = wait_stats()
                    result = api('POST', '/request-logs/cleanup', {
                        'dry_run': False, 'confirmation_token': plan['confirmation_token']})[2]
                    assert result['affected_rows'] > 0 and result['freed_bytes'] > 0
                    replay = api('POST', '/request-logs/cleanup', {
                        'dry_run': False, 'confirmation_token': plan['confirmation_token']}, expected=409)[2]
                    assert replay['error']['code'] == 'cleanup_confirmation_expired'
                    cleared = detail(target_id)
                    assert cleared['record']['id'] == target_id
                    assert all(not field['available'] for field in fields(cleared, 'response_body'))
                    api('GET', field_path(latest, target_field, 'download'), expected=410)
                    assert any(field['available'] for field in fields(detail(after_preview['record']['id']), 'response_body'))
                    assert stats_count() == before_stats
                    remaining = detail(after_preview['record']['id'])
                    remaining_field = next(field for field in fields(remaining, 'response_body') if field['available'])
                    expired_plan = api('POST', '/request-logs/cleanup', {
                        'dry_run': True, 'action': 'clear_fields', 'fields': ['response_body'],
                        'filters': {'ids': [remaining['record']['id']]}})[2]
                    assert expired_plan['matched_rows'] == 1
                    # Only this isolated fixture index is modified; avoid a five-minute sleep.
                    with closing(sqlite3.connect(root / 'request-logs/index.sqlite')) as connection:
                        connection.execute('UPDATE cleanup_plans SET expires_at_ms=0 WHERE token=?',
                                           (expired_plan['confirmation_token'],))
                        connection.commit()
                    expired = api('POST', '/request-logs/cleanup', {
                        'dry_run': False, 'confirmation_token': expired_plan['confirmation_token']}, expected=409)[2]
                    assert expired['error']['code'] == 'cleanup_confirmation_expired'
                    api('GET', field_path(remaining, remaining_field))
                    assert stats_count() == before_stats
                    print('PASS cleanup tokens: replay and expiry return 409 without clearing content or statistics')
                    delete_plan = api('POST', '/request-logs/cleanup', {
                        'dry_run': True, 'action': 'delete_rows', 'filters': {'ids': [target_id]}})[2]
                    assert delete_plan['matched_rows'] == 1
                    api('POST', '/request-logs/cleanup', {'dry_run': False, 'confirmation_token': delete_plan['confirmation_token']})
                    api('GET', '/request-logs/' + target_id, expected=404)
                    assert stats_count() == before_stats
                    print('PASS cleanup: dry-run is read-only, frozen targets exclude new requests, clear/delete are separate and statistics survive')

                    current = api('GET', '/log-settings')[2]
                    api('PUT', '/log-settings', {'revision': snapshot['revision'], 'settings': current['settings']}, expected=409)
                    set_settings(lambda settings: settings.update(enabled=False))
                    count = api('GET', '/request-logs')[2]['total']
                    disabled_status, _, _, _ = client.request('POST', '/v1/chat/completions', {'model': 'public-model', 'messages': [{'role': 'user', 'content': 'logging disabled'}]},
                                   headers={'Authorization': 'Bearer caller-secret'}, authenticated=False)
                    assert disabled_status == 200
                    wait_stats()
                    assert api('GET', '/request-logs')[2]['total'] == count
                    stop_gateway(process)
                    process = start()
                    client = login()
                    persisted = api('GET', '/log-settings')[2]
                    assert persisted['settings']['enabled'] is False and persisted['settings']['body_retention_hours'] == 24
                    assert api('GET', '/request-logs')[2]['total'] == count
                    print('PASS settings: disabling stops new rows, CAS is enforced, settings and logs survive restart')

                    stop_gateway(process)
                    process = None
                    env['DISABLE_DATABASE'] = 'true'
                    env['MIORA_REQUEST_LOG_DIR'] = str(root / 'disabled-request-logs')
                    process = start()
                    client = login()
                    stub = api('GET', '/log-settings')[2]
                    assert stub['backend_enabled'] is False and stub['status']['worker_alive'] is False
                    assert stub['status']['initialization_error'] is None
                    disabled = api('GET', '/request-logs', expected=503)[2]
                    assert disabled['error']['code'] == 'request_logs_disabled'
                    api('PUT', '/log-settings', {'revision': stub['revision'], 'settings': stub['settings']}, expected=503)
                    api('POST', '/request-logs/cleanup', {'dry_run': True, 'action': 'delete_rows', 'filters': {}}, expected=503)
                    assert not (root / 'disabled-request-logs').exists()
                    status, _, _, _ = client.request('POST', '/v1/chat/completions', {
                        'model': 'public-model', 'messages': [{'role': 'user', 'content': 'disabled backend'}]},
                        headers={'Authorization': 'Bearer caller-secret'}, authenticated=False)
                    assert status == 200
                    print('PASS disabled backend: settings are explicit, storage APIs return 503, no log directory is created, inference succeeds')

                    stop_gateway(process)
                    process = None
                    env['DISABLE_DATABASE'] = 'false'
                    blocked = root / 'request-logs-not-a-directory'
                    blocked.write_text('keep this fixture file', encoding='utf-8')
                    env['MIORA_REQUEST_LOG_DIR'] = str(blocked)
                    process = start()
                    client = login()
                    unavailable = api('GET', '/log-settings')[2]
                    assert unavailable['backend_enabled'] is False and unavailable['status']['worker_alive'] is False
                    assert unavailable['status']['initialization_error']
                    failure = api('GET', '/request-logs', expected=503)[2]
                    assert failure['error']['code'] == 'request_logs_unavailable'
                    api('PUT', '/log-settings', {'revision': unavailable['revision'], 'settings': unavailable['settings']}, expected=503)
                    before = stats_count()
                    status, _, _, _ = client.request('POST', '/v1/chat/completions', {
                        'model': 'public-model', 'messages': [{'role': 'user', 'content': 'unavailable log store'}]},
                        headers={'Authorization': 'Bearer caller-secret'}, authenticated=False)
                    assert status == 200
                    deadline = time.monotonic() + 5
                    while stats_count() != before + 1 and time.monotonic() < deadline:
                        time.sleep(.05)
                    assert stats_count() == before + 1
                    assert blocked.read_text(encoding='utf-8') == 'keep this fixture file'
                    print('PASS initialization failure: real API reports unavailable, inference and original statistics still work')
                except Exception:
                    if process is not None:
                        stop_gateway(process)
                    print((root / 'runtime.log').read_text(encoding='utf-8', errors='replace')[-7000:], file=sys.stderr)
                    raise
                finally:
                    if process is not None:
                        stop_gateway(process)
    finally:
        upstream.shutdown()
        upstream.server_close()


if __name__ == '__main__':
    verify(Path(sys.argv[1]).resolve())
