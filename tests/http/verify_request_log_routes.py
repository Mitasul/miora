#!/usr/bin/env python3
"""Focused capture integration for native Responses and failing upstream routes."""
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

from verify_request_logs import ALL_FIELDS, Client, free_port
from http_fixture_process import wait_for_gateway, stop_gateway


class Upstream(http.server.BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'

    def log_message(self, *_):
        pass

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        if self.path.endswith('/moderations'):
            raw = json.dumps({'id': 'moderation_fixture', 'model': 'omni-moderation-latest',
                              'results': [{'flagged': True}]}).encode()
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
        elif self.path.endswith('/responses'):
            events = [
                {'type': 'response.created', 'response': {'id': 'resp_fixture', 'model': 'native-wire-model', 'status': 'in_progress'}},
                {'type': 'response.output_text.delta', 'delta': 'native output'},
                {'type': 'response.completed', 'response': {'id': 'resp_fixture', 'model': 'native-wire-model', 'status': 'completed',
                    'output': [{'type': 'message', 'role': 'assistant', 'content': [{'type': 'output_text', 'text': 'native output'}]}]}},
            ]
            raw = ''.join(f"event: {event['type']}\ndata: {json.dumps(event)}\n\n" for event in events).encode()
            self.server.native_wire = raw
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
        else:
            raw = json.dumps({'error': {'message': 'fixture upstream failure', 'api_key': 'error-body-secret'}}).encode()
            self.send_response(503)
            self.send_header('Content-Type', 'application/json')
        self.send_header('X-Api-Key', 'upstream-header-secret')
        self.send_header('Content-Length', str(len(raw)))
        self.send_header('Connection', 'close')
        self.end_headers()
        self.wfile.write(raw)
        self.wfile.flush()
        self.close_connection = True


def verify(binary):
    upstream = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Upstream)
    threading.Thread(target=upstream.serve_forever, daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix='miora-log-routes-') as directory:
            root = Path(directory)
            executable = root / ('gateway.exe' if os.name == 'nt' else 'gateway')
            shutil.copy2(binary, executable)
            config = {'providers': [
                {'provider': 'native', 'engine': 'gpt', 'base_url': f'http://127.0.0.1:{upstream.server_port}/v1/responses',
                 'api': 'native-provider-secret', 'model': [{'native-upstream': 'native-public'}]},
                {'provider': 'failure', 'engine': 'gpt', 'base_url': f'http://127.0.0.1:{upstream.server_port}/v1/chat/completions',
                 'api': 'failed-provider-secret', 'model': ['failure-model']},
                {'provider': 'moderation', 'engine': 'gpt', 'base_url': f'http://127.0.0.1:{upstream.server_port}/v1/moderations',
                 'api': 'moderation-provider-secret', 'model': ['omni-moderation-latest']},
            ], 'api_keys': [{'api': 'caller-secret', 'role': 'user', 'model': ['all']},
                           {'api': 'moderated-caller-secret', 'role': 'user', 'model': ['all'],
                            'preferences': {'ENABLE_MODERATION': True}}],
                'preferences': {'AUTO_RETRY': False}}
            (root / 'api.json').write_text(json.dumps(config), encoding='utf-8')
            port = free_port()
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
                    client = Client(origin)
                    status, headers, session, _ = client.request('POST', '/admin/api/login', {'username': 'admin', 'password': password})
                    assert status == 200, session
                    client.cookie = headers['set-cookie'].split(';', 1)[0]
                    client.csrf = session['csrf_token']

                    def api(method, path, data=None):
                        result = client.request(method, '/admin/api' + path, data, headers={'X-CSRF-Token': client.csrf})
                        assert result[0] == 200, result[:3]
                        return result[2]

                    settings = api('GET', '/log-settings')
                    settings['settings']['capture_fields'] = ALL_FIELDS
                    settings['settings']['body_retention_hours'] = 24
                    api('PUT', '/log-settings', {'revision': settings['revision'], 'settings': settings['settings']})

                    def await_record(request_id):
                        deadline = time.monotonic() + 10
                        while time.monotonic() < deadline:
                            rows = api('GET', '/request-logs?' + urllib.parse.urlencode({'request_id': request_id}))['rows']
                            if rows and not rows[0]['active']:
                                return api('GET', '/request-logs/' + rows[0]['id'])
                            time.sleep(.05)
                        raise AssertionError('capture did not finish: ' + request_id)

                    def contents(document):
                        content = {}
                        for field in document['fields']:
                            if not field['available']:
                                continue
                            query = {'field': field['field']}
                            if field['attempt'] is not None:
                                query['attempt'] = field['attempt']
                            path = '/request-logs/' + document['record']['id'] + '/content?' + urllib.parse.urlencode(query)
                            content.setdefault(field['field'], []).append(api('GET', path)['content'])
                        return content

                    request_id = 'a1' * 16
                    status, _, _, raw = client.request('POST', '/v1/responses',
                        {'model': 'native-public', 'input': 'native fixture', 'stream': True, 'metadata': {'api_key': 'native-body-secret'}},
                        headers={'Authorization': 'Bearer caller-secret', 'X-Request-ID': request_id}, authenticated=False)
                    assert status == 200 and raw == upstream.native_wire, (status, raw)
                    native = await_record(request_id)
                    record = native['record']
                    assert record['status'] == 200 and record['relay_complete'] is True, record
                    assert record['channel'] == 'native' and record['requested_model'] == 'native-public', record
                    assert record['sent_model'] == 'native-upstream' and record['response_model'] == 'native-wire-model', record
                    native_content = contents(native)
                    assert set(native_content) == set(ALL_FIELDS), native_content.keys()
                    assert json.loads(native_content['upstream_request_body'][0])['model'] == 'native-upstream'
                    assert 'native output' in native_content['upstream_response_body'][0]
                    text = json.dumps(native_content)
                    for secret in ['caller-secret', 'native-provider-secret', 'native-body-secret', 'upstream-header-secret']:
                        assert secret not in text, secret
                    print('PASS native Responses: all four stages, exact relay bytes, model names, secret masking, no-usage success')

                    request_id = 'b2' * 16
                    status, _, _, _ = client.request('POST', '/v1/chat/completions',
                        {'model': 'failure-model', 'messages': [{'role': 'user', 'content': 'fail fixture'}]},
                        headers={'Authorization': 'Bearer caller-secret', 'X-Request-ID': request_id}, authenticated=False)
                    assert status >= 400, status
                    failure = await_record(request_id)
                    record = failure['record']
                    assert record['status'] == status and record['attempt_count'] >= 1, record
                    failure_content = contents(failure)
                    assert 'upstream_response_body' in failure_content and 'response_body' in failure_content, failure
                    assert 'fixture upstream failure' in failure_content['upstream_response_body'][0]
                    for secret in ['failed-provider-secret', 'caller-secret', 'error-body-secret', 'upstream-header-secret']:
                        assert secret not in json.dumps(failure_content), secret
                    print('PASS upstream HTTP failure: actual failure status and attempt retained, error content sanitized')

                    request_id = 'c3' * 16
                    status, _, _, _ = client.request('POST', '/v1/chat/completions',
                        {'model': 'failure-model', 'stream': True, 'messages': [{'role': 'user', 'content': 'flagged fixture'}]},
                        headers={'Authorization': 'Bearer moderated-caller-secret', 'X-Request-ID': request_id}, authenticated=False)
                    assert status == 400, status
                    moderated = await_record(request_id)
                    record = moderated['record']
                    assert record['requested_model'] == 'failure-model' and record['stream'] is True, record
                    assert record['attempt_count'] == 1 and record['status'] == 400, record
                    attempt = record['attempts'][0]
                    assert attempt['phase'] == 'moderation' and attempt['endpoint'] == '/v1/moderations', attempt
                    assert attempt['sent_model'] == 'omni-moderation-latest', attempt
                    print('PASS moderation refusal: client model and stream preserved; preflight attempt identified separately')

                    request_id = 'd4' * 16
                    status, _, _, _ = client.request('POST', '/v1/moderations/',
                        {'model': 'omni-moderation-latest', 'input': 'direct moderation fixture'},
                        headers={'Authorization': 'Bearer caller-secret', 'X-Request-ID': request_id}, authenticated=False)
                    assert status == 200, status
                    direct = await_record(request_id)['record']
                    assert direct['channel'] == 'moderation' and direct['sent_model'] == 'omni-moderation-latest', direct
                    assert direct['attempts'][0]['phase'] == 'inference', direct
                    print('PASS direct moderation with trailing slash: canonical endpoint is not mistaken for auxiliary preflight')
                except Exception:
                    stop_gateway(process)
                    print((root / 'runtime.log').read_text(encoding='utf-8', errors='replace')[-7000:], file=sys.stderr)
                    raise
                finally:
                    stop_gateway(process)
    finally:
        upstream.shutdown()
        upstream.server_close()


if __name__ == '__main__':
    verify(Path(sys.argv[1]).resolve())
