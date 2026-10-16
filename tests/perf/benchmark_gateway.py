#!/usr/bin/env python3
"""Repeatable loopback-only gateway benchmark; no real supplier or preview data.

Use the SAME build profile for both binaries. Numbers include a Python local
supplier and are not WAN/production capacity estimates. Capture failures and
writer drops are reported so discarded logging is never counted as a free win.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
from contextlib import closing
import http.client
import http.server
import json
import os
from pathlib import Path
import secrets
import shutil
import socket
import sqlite3
import statistics
import subprocess
import sys
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tests/http'))
from verify_admin import Client
from http_fixture_process import stop_gateway, wait_for_gateway


class Upstream(http.server.BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'

    def setup(self):
        super().setup()
        self.connection.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)

    def log_message(self, *_):
        pass

    def handle(self):
        try:
            super().handle()
        except (ConnectionResetError, BrokenPipeError):
            pass

    def do_POST(self):
        payload = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        streaming = payload.get('stream', False)
        raw = self.server.sse if streaming else self.server.json_body
        self.send_response(200)
        self.send_header('Content-Type', 'text/event-stream' if streaming else 'application/json')
        self.send_header('Content-Length', str(len(raw)))
        self.end_headers()
        if streaming:
            # Fixed writes; TCP may coalesce them. A deterministic chunk-level
            # allocation check is separate from this end-to-end measurement.
            for offset in range(0, len(raw), 512):
                self.wfile.write(raw[offset:offset + 512])
        else:
            self.wfile.write(raw)
        self.wfile.flush()


def free_port():
    with socket.socket() as connection:
        connection.bind(('127.0.0.1', 0))
        return connection.getsockname()[1]


def install_plugin_fixture(directory, count, endpoint):
    entries = {}
    for index in range(count):
        name = f'bench-plugin-{index:04}'
        manifest = {'id': name, 'name': name, 'version': '1.0', 'api_version': 1,
                    'capabilities': {'endpoints': ['/v1/chat/completions'], 'streaming': True},
                    'models': [], 'config_schema': []}
        version = {'revision': 1, 'manifest': manifest, 'endpoint': endpoint,
                   'config': {}, 'service_auth': {'type': 'none', 'token': ''}}
        entries[name] = {'enabled': True, 'revision': 1, 'active': 1, 'versions': [version]}
    directory.mkdir()
    (directory / 'revision-00000000000000000001.json').write_text(json.dumps({
        'format': 1, 'generation': 1, 'entries': entries,
        'interceptor_revision': 0, 'interceptors': []}), encoding='utf-8')


def percentile(samples, fraction):
    values = sorted(samples)
    return values[min(len(values) - 1, int((len(values) - 1) * fraction))]


def burst(port, count, concurrency, streaming, pacing_ms=0):
    payload = json.dumps({'model': 'bench-model', 'stream': streaming,
                          'messages': [{'role': 'user', 'content': 'local benchmark'}]}).encode()
    local = threading.local()
    connections = []
    lock = threading.Lock()

    def call(_):
        if not hasattr(local, 'connection'):
            local.connection = http.client.HTTPConnection('127.0.0.1', port, timeout=30)
            with lock:
                connections.append(local.connection)
        started = time.perf_counter()
        local.connection.request('POST', '/v1/chat/completions', payload,
                                 {'Authorization': 'Bearer bench-caller', 'Content-Type': 'application/json'})
        response = local.connection.getresponse()
        raw = response.read()
        duration = (time.perf_counter() - started) * 1000
        if response.status != 200:
            raise AssertionError((response.status, raw[:200]))
        if streaming and b'[DONE]' not in raw:
            raise AssertionError('stream did not finish')
        if pacing_ms:
            time.sleep(pacing_ms / 1000)
        return duration, len(raw)

    started = time.perf_counter()
    try:
        with ThreadPoolExecutor(max_workers=concurrency) as pool:
            results = list(pool.map(call, range(count)))
    finally:
        for connection in connections:
            connection.close()
    elapsed = time.perf_counter() - started
    samples = [item[0] for item in results]
    return {'requests': count, 'concurrency': concurrency, 'elapsed_s': elapsed,
            'requests_per_s': count / elapsed, 'p50_ms': statistics.median(samples),
            'p95_ms': percentile(samples, .95), 'response_bytes': sum(item[1] for item in results)}


def benchmark(binary, output, rounds=3, requests=160, concurrency=4, plugins=256, only_logging=None, pacing_ms=0):
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Upstream)
    server.json_body = json.dumps({'id': 'bench', 'object': 'chat.completion', 'model': 'bench-model',
        'choices': [{'index': 0, 'message': {'role': 'assistant', 'content': 'local output'}, 'finish_reason': 'stop'}]}).encode()
    chunk = {'id': 'bench', 'object': 'chat.completion.chunk', 'model': 'bench-model',
             'choices': [{'index': 0, 'delta': {'content': 'local text ' * 16}, 'finish_reason': None}]}
    server.sse = (('data: ' + json.dumps(chunk) + '\n\n') * 128 + 'data: [DONE]\n\n').encode()
    threading.Thread(target=server.serve_forever, daemon=True).start()
    report = {'binary': str(binary), 'build_profile_required': 'same for compared runs',
              'supplier': 'loopback Python fixture', 'rounds': rounds,
              'pacing_ms_per_worker_after_response': pacing_ms,
              'throughput_includes_configured_pacing': True, 'scenarios': []}
    try:
        # Keep all test files under the workspace; no external temporary cleanup.
        with tempfile.TemporaryDirectory(prefix='bench-', dir=ROOT / '.artifacts/performance') as directory:
            root = Path(directory)
            executable = root / 'gateway.exe'
            shutil.copy2(binary, executable)
            for plugin_count in ([plugins] if only_logging and only_logging != 'off' else [0, plugins]):
                case = root / f'plugins-{plugin_count}'
                case.mkdir()
                port = free_port()
                origin = f'http://127.0.0.1:{port}'
                password = secrets.token_urlsafe(24)
                supplier = f'http://127.0.0.1:{server.server_port}'
                install_plugin_fixture(case / 'plugins', plugin_count, supplier)
                config = {'providers': [{'provider': 'bench', 'engine': 'gpt',
                           'base_url': supplier + '/v1/chat/completions', 'api': 'bench-upstream', 'model': ['bench-model']}],
                          'api_keys': [{'api': 'bench-caller', 'role': 'user', 'model': ['all'],
                                        'preferences': {'AUTO_RETRY': False}}]}
                (case / 'api.json').write_text(json.dumps(config), encoding='utf-8')
                env = {key: value for key, value in os.environ.items()
                       if not key.startswith(('MIORA_', 'FACTS_S3_', 'UNI_API_CONTROL_RESTORE')) and key != 'CONFIG_URL'}
                env.update(PORT=str(port), DISABLE_DATABASE='false', DB_TYPE='sqlite', DB_PATH=str(case / 'stats.db'),
                           MIORA_ADMIN_USERNAME='admin', MIORA_ADMIN_PASSWORD=password, MIORA_ADMIN_ORIGIN=origin,
                           MIORA_ADMIN_SECURE_COOKIE='false', UNI_API_CONFIG_PATH=str(case / 'api.json'),
                           RUST_RESPONSES_CONFIG_SNAPSHOT_PATH=str(case / 'snapshot.json'),
                           UNI_API_SHARED_MEMORY_RESERVATION_PATH=str(case / 'ledger'),
                           RUST_REQUEST_SPOOL_DIRECTORY=str(case / 'spool'), MIORA_PLUGIN_STORE_DIR=str(case / 'plugins'),
                           MIORA_PLUGIN_ALLOW_LOOPBACK_HTTP='true', MIORA_REQUEST_LOG_DIR=str(case / 'request-logs'),
                           STDOUT_REQUEST_SUMMARY_LOG_ENABLED='false', NO_PROXY='127.0.0.1,localhost')
                with (case / 'runtime.log').open('wb') as log:
                    process = subprocess.Popen([str(executable)], cwd=case, env=env, stdout=log, stderr=log, creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
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

                        def settle():
                            deadline = time.monotonic() + 30
                            while time.monotonic() < deadline:
                                snapshot = api('GET', '/log-settings')
                                if snapshot['status']['active_records'] == 0 and snapshot['status']['queued_bytes'] == 0:
                                    return snapshot['status']
                                time.sleep(.02)
                            raise AssertionError('log writer did not settle')

                        modes = [('off', False)] if plugin_count == 0 else [('off', False), ('summary', False), ('summary', True), ('capture', True)]
                        for mode, streaming in modes:
                            if only_logging and mode != only_logging:
                                continue
                            snapshot = api('GET', '/log-settings')
                            settings = snapshot['settings']
                            settings['enabled'] = mode != 'off'
                            settings['body_retention_hours'] = 1 if mode == 'capture' else 0
                            settings['capture_fields'] = ['upstream_response_body', 'response_body'] if mode == 'capture' else []
                            settings['limits'] = {'per_request_bytes': 262144, 'total_body_bytes': 268435456}
                            api('PUT', '/log-settings', {'revision': snapshot['revision'], 'settings': settings})
                            burst(port, 12, concurrency, streaming, pacing_ms)
                            settle()
                            samples = []
                            for _ in range(rounds):
                                before = settle()
                                sample = burst(port, requests, concurrency, streaming, pacing_ms)
                                drain_started = time.perf_counter()
                                after = settle()
                                sample['log_drain_s'] = time.perf_counter() - drain_started
                                sample['capture_dropped_records'] = after['dropped_content_records'] - before['dropped_content_records']
                                sample['writer_failures'] = after['write_failures'] - before['write_failures']
                                samples.append(sample)
                            with closing(sqlite3.connect(case / 'request-logs/index.sqlite')) as connection:
                                failure_reasons = dict(connection.execute("SELECT json_extract(summary_json, '$.capture_failure'), COUNT(*) FROM request_log_rows WHERE json_extract(summary_json, '$.capture_failure') IS NOT NULL GROUP BY json_extract(summary_json, '$.capture_failure')").fetchall())
                                body_fields = [dict(zip(['field', 'availability', 'complete', 'capture_state', 'count', 'saved_bytes', 'observed_bytes'], row))
                                               for row in connection.execute("SELECT field,availability,complete,capture_state,COUNT(*),SUM(saved_bytes),SUM(observed_bytes) FROM request_log_fields WHERE field IN ('upstream_response_body','response_body') GROUP BY field,availability,complete,capture_state").fetchall()]
                                row_count = connection.execute('SELECT COUNT(*) FROM request_log_rows').fetchone()[0]
                            item = {'plugins': plugin_count, 'logging': mode, 'stream': streaming,
                                    'samples': samples, 'capture_failure_rows_cumulative': sum(failure_reasons.values()),
                                    'capture_failure_reasons_cumulative': failure_reasons,
                                    'log_rows_cumulative': row_count, 'body_field_states_cumulative': body_fields,
                                    'median_requests_per_s': statistics.median(sample['requests_per_s'] for sample in samples),
                                    'median_p50_ms': statistics.median(sample['p50_ms'] for sample in samples),
                                    'median_p95_ms': statistics.median(sample['p95_ms'] for sample in samples)}
                            report['scenarios'].append(item)
                            print(json.dumps({key: value for key, value in item.items() if key != 'samples'}), flush=True)
                    except Exception:
                        stop_gateway(process)
                        print((case / 'runtime.log').read_text(encoding='utf-8', errors='replace')[-5000:], file=sys.stderr)
                        raise
                    finally:
                        stop_gateway(process)
    finally:
        server.shutdown()
        server.server_close()
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2), encoding='utf-8')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--rounds', type=int, default=3)
    parser.add_argument('--requests', type=int, default=160)
    parser.add_argument('--concurrency', type=int, default=4)
    parser.add_argument('--plugins', type=int, default=256)
    parser.add_argument('--only-logging', choices=['off', 'summary', 'capture'])
    parser.add_argument('--pacing-ms', type=float, default=0,
                        help='Optional per-worker pause after response; latency excludes pause, throughput includes it')
    args = parser.parse_args()
    if min(args.rounds, args.requests, args.concurrency, args.plugins) < 1:
        parser.error('rounds, requests, concurrency and plugins must be positive')
    if args.pacing_ms < 0:
        parser.error('pacing-ms must not be negative')
    benchmark(args.binary.resolve(), args.output.resolve(), args.rounds, args.requests, args.concurrency, args.plugins,
              args.only_logging, args.pacing_ms)
