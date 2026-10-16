#!/usr/bin/env python3
"""Parameter-policy slice: compare real upstream requests with admin previews.

Only a disposable gateway and a local HTTP fixture are used. No supplier calls,
file hashes, browser asset checks or unrelated regression suites are involved.
"""
import copy
import http.server
import json
import os
from pathlib import Path
import secrets
import shutil
import socket
import subprocess
import sys
import tempfile
import threading

from http_fixture_process import wait_for_gateway, stop_gateway
from verify_admin import Client


class Upstream(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        payload = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        self.server.hits.append({
            'path': self.path,
            'body': payload,
            'headers': {key.lower(): value for key, value in self.headers.items()},
        })
        if self.path.endswith('/responses'):
            value = {
                'id': 'resp_policy', 'object': 'response', 'status': 'completed',
                'model': payload['model'],
                'output': [{'id': 'msg_policy', 'type': 'message', 'role': 'assistant',
                            'status': 'completed', 'content': [{'type': 'output_text', 'text': 'fixture reply'}]}],
                'usage': {'input_tokens': 1, 'output_tokens': 2, 'total_tokens': 3},
            }
        else:
            value = {
                'id': 'chat_policy', 'object': 'chat.completion', 'model': payload['model'],
                'choices': [{'index': 0, 'message': {'role': 'assistant', 'content': 'fixture reply'}, 'finish_reason': 'stop'}],
                'usage': {'prompt_tokens': 1, 'completion_tokens': 2, 'total_tokens': 3},
            }
        raw = json.dumps(value).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)


def free_port():
    with socket.socket() as connection:
        connection.bind(('127.0.0.1', 0))
        return connection.getsockname()[1]


def policy(model, token_field):
    return {
        'defaults': {'top_p': .7, 'keep_zero': 1, 'keep_false': True,
                     'keep_blank': 'default', 'keep_null': 'default',
                     'nested': {'from_default': True}},
        'channel': {'set': {'temperature': 0, token_field: 128,
                            'nested': {'from_channel': False, 'array': [1, 2]},
                            'explicit_null': None, 'explicit_blank': ''},
                    'remove': ['/seed', '/nested/delete_me']},
        'models': [{'match': {'requested_model': model},
                    'set': {'temperature': .4, 'nested': {'from_model': True, 'array': [3]}},
                    'remove': ['/top_p']}],
        'headers': [{'name': 'X-Policy', 'mode': 'set', 'value': 'applied', 'enabled': True, 'secret': False},
                    {'name': 'X-Policy-Secret', 'mode': 'set', 'value': 'supplier-secret-fixture', 'enabled': True, 'secret': True},
                    {'name': 'Authorization', 'mode': 'set', 'value': 'Bearer header-secret-fixture', 'enabled': True, 'secret': False}],
        'system_prompt': {'mode': 'prepend', 'value': 'channel instruction'},
        'future_extension': {'keep': [1, 2, 3]},
    }


def verify(binary):
    upstream = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Upstream)
    upstream.hits = []
    thread = threading.Thread(target=upstream.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix='miora-policy-') as directory:
            root = Path(directory)
            executable = root / ('gateway.exe' if os.name == 'nt' else 'gateway')
            shutil.copy2(binary, executable)
            config = {'providers': [
                {'provider': 'chat', 'engine': 'gpt', 'base_url': f'http://127.0.0.1:{upstream.server_port}/chat/v1/chat/completions',
                 'api': 'stored-upstream-key', 'model': [{'upstream-chat': 'public-chat'}],
                 'preferences': {'parameter_policy': policy('public-chat', 'max_tokens')}},
                {'provider': 'native', 'engine': 'gpt', 'base_url': f'http://127.0.0.1:{upstream.server_port}/native/v1/responses',
                 'api': 'stored-upstream-key', 'model': [{'upstream-native': 'public-native'}],
                 'preferences': {'parameter_policy': policy('public-native', 'max_output_tokens')}},
            ], 'api_keys': [{'api': 'fixture-caller', 'role': 'user', 'model': ['all'],
                             'preferences': {'AUTO_RETRY': False}}]}
            (root / 'api.json').write_text(json.dumps(config), encoding='utf-8')
            port = free_port()
            origin = f'http://127.0.0.1:{port}'
            password = secrets.token_urlsafe(24)
            env = {key: value for key, value in os.environ.items()
                   if not key.startswith(('MIORA_', 'FACTS_S3_', 'UNI_API_CONTROL_RESTORE')) and key != 'CONFIG_URL'}
            env.update(PORT=str(port), DISABLE_DATABASE='true',
                       MIORA_ADMIN_USERNAME='admin', MIORA_ADMIN_PASSWORD=password,
                       MIORA_ADMIN_SECURE_COOKIE='false', MIORA_ADMIN_ORIGIN=origin,
                       UNI_API_CONFIG_PATH=str(root / 'api.json'),
                       RUST_RESPONSES_CONFIG_SNAPSHOT_PATH=str(root / 'snapshot.json'),
                       UNI_API_SHARED_MEMORY_RESERVATION_PATH=str(root / 'ledger'),
                       RUST_REQUEST_SPOOL_DIRECTORY=str(root / 'spool'),
                       MIORA_PLUGIN_STORE_DIR=str(root / 'plugins'), NO_PROXY='127.0.0.1,localhost')
            with (root / 'runtime.log').open('wb') as log:
                process = subprocess.Popen([str(executable)], cwd=root, env=env, stdout=log, stderr=log, creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
                try:
                    wait_for_gateway(process, port)
                    client = Client(origin)
                    status, headers, session, _ = client.request('POST', '/admin/api/login',
                        {'username': 'admin', 'password': password})
                    assert status == 200, (status, session)
                    client.cookie = headers['set-cookie'].split(';', 1)[0]
                    client.csrf = session['csrf_token']

                    def admin(method, path, body=None, expected=200):
                        status, _, data, _ = client.request(method, '/admin/api' + path, body,
                            headers={'X-CSRF-Token': client.csrf})
                        assert status == expected, (method, path, status, data)
                        return data

                    def send(endpoint, body):
                        status, _, data, _ = client.request('POST', endpoint, body,
                            headers={'Authorization': 'Bearer fixture-caller'}, authenticated=False)
                        assert status == 200, (status, data)
                        return upstream.hits[-1]

                    for provider, endpoint, model, sent_model, token_field in (
                        ('chat', '/v1/chat/completions', 'public-chat', 'upstream-chat', 'max_tokens'),
                        ('native', '/v1/responses', 'public-native', 'upstream-native', 'max_output_tokens'),
                    ):
                        body = {'model': model, 'stream': False, 'temperature': .8, 'top_p': .9,
                                'keep_zero': 0, 'keep_false': False, 'keep_blank': '', 'keep_null': None,
                                'nested': {'from_client': True, 'array': [9], 'delete_me': 'remove'}, 'seed': 1}
                        if provider == 'chat':
                            body['messages'] = [{'role': 'system', 'content': 'client instruction'},
                                                {'role': 'user', 'content': 'fixture request'}]
                        else:
                            body.update(input='fixture request', instructions='client instruction')
                        before = len(upstream.hits)
                        preview = admin('POST', '/channels/preview', {
                            'provider': provider, 'channel': {},
                            'request': {'endpoint': endpoint, 'model': model, 'stream': False, 'body': copy.deepcopy(body)},
                        })
                        assert len(upstream.hits) == before, 'preview sent an upstream request'
                        assert preview['scope'] == 'upstream_request', preview
                        visible = json.dumps(preview)
                        assert 'stored-upstream-key' not in visible and 'supplier-secret-fixture' not in visible and 'header-secret-fixture' not in visible
                        actual = send(endpoint, body)
                        assert actual['body'] == preview['body'], (provider, actual['body'], preview['body'])
                        wire = actual['body']
                        assert wire['model'] == sent_model
                        assert wire['temperature'] == .4 and 'top_p' not in wire and 'seed' not in wire
                        assert wire['keep_zero'] == 0 and wire['keep_false'] is False
                        assert wire['keep_blank'] == '' and wire['keep_null'] is None
                        assert wire['explicit_null'] is None and wire['explicit_blank'] == ''
                        assert wire['nested'] == {'from_client': True, 'from_default': True,
                                                   'from_channel': False, 'from_model': True, 'array': [3]}
                        assert wire[token_field] == 128
                        assert len(set(wire) & {'max_tokens', 'max_completion_tokens', 'max_output_tokens'}) == 1
                        assert actual['headers']['x-policy'] == 'applied'
                        assert actual['headers']['x-policy-secret'] == 'supplier-secret-fixture'
                        assert actual['headers']['authorization'] == 'Bearer header-secret-fixture'
                        if provider == 'chat':
                            system = [item['content'] for item in wire['messages'] if item['role'] == 'system']
                            assert 'channel instruction' in '\n'.join(system) and 'client instruction' in '\n'.join(system)
                            assert wire['messages'][-1] == body['messages'][-1]
                        else:
                            assert wire['instructions'].startswith('channel instruction') and 'client instruction' in wire['instructions']
                        print(f'PASS {provider}: real wire equals preview; tri-state/defaults/model/remove/header/system/token policy')

                    detail = admin('GET', '/channels/detail?provider=chat')
                    assert 'supplier-secret-fixture' not in json.dumps(detail) and 'header-secret-fixture' not in json.dumps(detail)
                    inherited = {'headers': [{'name': 'X-Policy-Secret', 'mode': 'set', 'enabled': True, 'secret': True}]}
                    admin('PATCH', '/channels', {'revision': detail['revision'], 'provider': 'chat',
                          'patch': {'preferences': {'parameter_policy': inherited}}})
                    request = {'model': 'public-chat', 'messages': [{'role': 'user', 'content': 'inherit'}], 'temperature': .8}
                    wire = send('/v1/chat/completions', request)
                    assert wire['body']['temperature'] == .8, 'removed override returned during PATCH merge'
                    assert wire['headers']['x-policy-secret'] == 'supplier-secret-fixture'
                    detail = admin('GET', '/channels/detail?provider=chat')
                    assert detail['effective']['preferences']['parameter_policy']['future_extension'] == {'keep': [1, 2, 3]}
                    inherited['headers'][0]['value'] = ''
                    admin('PATCH', '/channels', {'revision': detail['revision'], 'provider': 'chat',
                          'patch': {'preferences': {'parameter_policy': inherited}}})
                    wire = send('/v1/chat/completions', request)
                    assert 'x-policy-secret' in wire['headers'] and wire['headers']['x-policy-secret'] == ''
                    admin('PATCH', '/channels', {'revision': detail['revision'], 'provider': 'chat',
                          'patch': {'preferences': {'parameter_policy': {}}}}, expected=409)
                    print('PASS save: inherit removes old override, unknown fields survive, omitted secret retained, empty secret explicit, stale revision rejected')
                except Exception:
                    stop_gateway(process)
                    print((root / 'runtime.log').read_text(encoding='utf-8', errors='replace')[-5000:], file=sys.stderr)
                    raise
                finally:
                    stop_gateway(process)
    finally:
        upstream.shutdown()
        upstream.server_close()


if __name__ == '__main__':
    verify(Path(sys.argv[1]).resolve())
