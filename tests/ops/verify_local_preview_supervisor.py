#!/usr/bin/env python3
"""Exercise managed runtime settings through a real isolated local supervisor."""
from pathlib import Path
import hashlib
import json
import os
import secrets
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tests/http'))
from verify_admin import Client
from verify_request_logs import free_port


def verify(binary):
    with tempfile.TemporaryDirectory(prefix='miora-runtime-supervisor-') as directory:
        root = Path(directory)
        shutil.copy2(binary, root / 'uni-api-front.exe')
        port = free_port()
        password = secrets.token_urlsafe(24)
        salt = secrets.token_bytes(16)
        derived = hashlib.pbkdf2_hmac('sha256', password.encode(), salt, 600000)
        settings = {'port': port, 'username': 'admin',
                    'password_hash': f'pbkdf2-sha256$600000${salt.hex()}${derived.hex()}'}
        (root / 'settings.json').write_text(json.dumps(settings), encoding='utf-8')
        (root / 'api.yaml').write_text('providers: []\napi_keys: []\n', encoding='utf-8')
        env = {key: value for key, value in os.environ.items()
               if not key.startswith(('MIORA_', 'FACTS_S3_', 'UNI_API_CONTROL_RESTORE')) and key != 'CONFIG_URL'}
        supervisor = None
        child_pid = None
        with (root / 'supervisor.log').open('wb') as log:
            try:
                supervisor = subprocess.Popen([sys.executable, str(ROOT / 'scripts/local_preview.py'), '--directory', str(root)],
                                              cwd=root, env=env, stdout=log, stderr=log, creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
                origin = f'http://127.0.0.1:{port}'
                deadline = time.monotonic() + 15
                while time.monotonic() < deadline:
                    if supervisor.poll() is not None:
                        raise AssertionError('supervisor exited before startup')
                    try:
                        with urllib.request.urlopen(origin + '/healthz', timeout=.3) as response:
                            if response.status == 200:
                                break
                    except OSError:
                        time.sleep(.05)
                else:
                    raise AssertionError('gateway did not become healthy')
                child_pid = int((root / 'server.pid').read_text())
                client = Client(origin)
                status, headers, session, _ = client.request('POST', '/admin/api/login', {'username': 'admin', 'password': password})
                assert status == 200, session
                client.cookie = headers['set-cookie'].split(';', 1)[0]
                client.csrf = session['csrf_token']
                status, _, snapshot, _ = client.request('GET', '/admin/api/log-settings')
                assert status == 200 and snapshot['backend_enabled'] and snapshot['runtime_rotation_managed'], snapshot
                runtime = root / 'runtime-logs'
                archive = runtime / 'runtime.2.log'
                archive.write_bytes(b'old runtime fixture')
                snapshot['settings']['runtime'] = {'file_max_bytes': 1024, 'retained_files': 1}
                status, _, saved, _ = client.request('PUT', '/admin/api/log-settings',
                    {'revision': snapshot['revision'], 'settings': snapshot['settings']},
                    headers={'X-CSRF-Token': client.csrf})
                assert status == 200, saved
                deadline = time.monotonic() + 4
                while archive.exists() and time.monotonic() < deadline:
                    time.sleep(.05)
                assert not archive.exists(), 'running supervisor did not read the new persisted policy'
                assert len(list(runtime.glob('runtime*.log'))) <= 1
                assert all(path.stat().st_size <= 1024 for path in runtime.glob('runtime*.log'))
                with urllib.request.urlopen(origin + '/healthz', timeout=1) as response:
                    assert response.status == 200
                print('PASS real supervisor: hashed login, managed runtime API, saved limits applied live, healthy gateway')
            except Exception:
                print((root / 'supervisor.log').read_text(encoding='utf-8', errors='replace'), file=sys.stderr)
                for path in (root / 'runtime-logs').glob('runtime*.log'):
                    print(path.read_text(encoding='utf-8', errors='replace')[-4000:], file=sys.stderr)
                raise
            finally:
                if child_pid is None and (root / 'server.pid').exists():
                    child_pid = int((root / 'server.pid').read_text())
                if child_pid:
                    # Only the process whose PID the isolated supervisor wrote.
                    if os.name == 'nt':
                        subprocess.run(['taskkill', '/PID', str(child_pid), '/F'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
                    else:
                        os.kill(child_pid, 15)
                if supervisor is not None:
                    try:
                        supervisor.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        supervisor.kill()
                        supervisor.wait(timeout=5)


if __name__ == '__main__':
    verify(Path(sys.argv[1]).resolve())
