#!/usr/bin/env python3
"""Run the local preview with bounded, live-configurable runtime log files.

The gateway remains the source of request-log settings. This supervisor only
owns the preview's stdout/stderr files, not request bodies or analytics data.
"""
from pathlib import Path
from contextlib import closing
import argparse
import json
import os
import queue
import sqlite3
import subprocess
import sys
import threading
import time

DEFAULT_FILE_BYTES = 10 * 1024 * 1024
DEFAULT_RETAINED_FILES = 3


class RotatingOutput:
    """retained_files includes runtime.log itself, not only its archives."""

    def __init__(self, directory, file_max_bytes=DEFAULT_FILE_BYTES,
                 retained_files=DEFAULT_RETAINED_FILES):
        self.directory = Path(directory)
        self.directory.mkdir(parents=True, exist_ok=True)
        self.current = self.directory / 'runtime.log'
        self.file_max_bytes = int(file_max_bytes)
        self.retained_files = int(retained_files)
        if self.file_max_bytes < 1 or self.retained_files < 1:
            raise ValueError('Runtime log size and total file count must be positive')
        self.file = self.current.open('ab', buffering=0)
        self.size = self.current.stat().st_size
        self._trim_archives()
        if self.size > self.file_max_bytes:
            self._rotate()

    def _archive(self, index):
        return self.directory / f'runtime.{index}.log'

    def _trim_archives(self):
        for path in self.directory.glob('runtime.*.log'):
            number = path.name.removeprefix('runtime.').removesuffix('.log')
            if number.isdigit() and (int(number) >= self.retained_files
                                     or path.stat().st_size > self.file_max_bytes):
                path.unlink()

    def configure(self, file_max_bytes, retained_files):
        values = int(file_max_bytes), int(retained_files)
        if values[0] < 1 or values[1] < 1:
            raise ValueError('Runtime log size and total file count must be positive')
        if values == (self.file_max_bytes, self.retained_files):
            return
        self.file_max_bytes, self.retained_files = values
        self._trim_archives()
        if self.size > self.file_max_bytes:
            self._rotate()

    def _rotate(self):
        self.file.close()
        if self.retained_files == 1:
            self.file = self.current.open('wb', buffering=0)
        else:
            oldest = self._archive(self.retained_files - 1)
            oldest.unlink(missing_ok=True)
            for index in range(self.retained_files - 2, 0, -1):
                path = self._archive(index)
                if path.exists():
                    os.replace(path, self._archive(index + 1))
            os.replace(self.current, self._archive(1))
            self.file = self.current.open('wb', buffering=0)
        self.size = 0
        self._trim_archives()

    def write(self, data):
        if self.file.closed:
            self.file = self.current.open('ab', buffering=0)
            self.size = self.current.stat().st_size
        view = memoryview(data)
        while view:
            if self.size >= self.file_max_bytes:
                self._rotate()
            count = min(len(view), self.file_max_bytes - self.size)
            written = self.file.write(view[:count])
            self.size += written
            view = view[written:]

    def close(self):
        self.file.close()


def runtime_policy(database):
    """Before gateway initialization, or during a DB write, keep the last policy."""
    try:
        with closing(sqlite3.connect(Path(database).resolve().as_uri() + '?mode=ro',
                                     uri=True, timeout=0.1)) as connection:
            row = connection.execute('SELECT config_json FROM log_settings WHERE id=1').fetchone()
        if row is None:
            return None
        settings = json.loads(row[0])['runtime']
        return int(settings['file_max_bytes']), int(settings['retained_files'])
    except sqlite3.Error:
        return None


class RuntimeLogPump:
    """A slow/full log disk must never stop draining the gateway's output pipe."""

    def __init__(self, output, database):
        self.output = output
        self.database = database
        self.queue = queue.Queue(maxsize=64)
        self.stopped = threading.Event()
        self.lock = threading.Lock()
        self.dropped_bytes = 0
        self.write_failures = 0
        self.thread = threading.Thread(target=self._run, daemon=True)
        self.thread.start()

    def _dropped(self, count):
        with self.lock:
            self.dropped_bytes += count

    def submit(self, data):
        try:
            self.queue.put_nowait(data)
        except queue.Full:
            self._dropped(len(data))

    def _run(self):
        last_refresh = 0.0
        last_error = None
        retry_write_at = 0.0
        try:
            while not self.stopped.is_set() or not self.queue.empty():
                try:
                    data = self.queue.get(timeout=0.1)
                except queue.Empty:
                    data = None
                now = time.monotonic()
                if now < retry_write_at:
                    if data is not None:
                        self._dropped(len(data))
                    continue
                try:
                    if now - last_refresh >= 1:
                        policy = runtime_policy(self.database)
                        if policy is not None:
                            self.output.configure(*policy)
                        last_refresh = now
                    if data is None:
                        continue
                    with self.lock:
                        dropped, failures = self.dropped_bytes, self.write_failures
                    if dropped or failures:
                        notice = {'event': 'runtime_log_capture_gap', 'dropped_bytes': dropped,
                                  'write_failures': failures, 'reason': 'backpressure_or_io'}
                        self.output.write(b'\n' + json.dumps(notice).encode() + b'\n')
                        with self.lock:
                            self.dropped_bytes -= dropped
                            self.write_failures -= failures
                    self.output.write(data)
                    last_error = None
                except OSError as error:
                    with self.lock:
                        self.write_failures += 1
                    message = f'Runtime log write failed: {error}'
                    if message != last_error:
                        try:
                            print(message, file=sys.stderr, flush=True)
                        except (OSError, ValueError):
                            pass
                    last_error = message
                    retry_write_at = now + 1
        finally:
            self.output.close()

    def close(self):
        self.stopped.set()
        # This is a best-effort runtime log drain, not a reason to hang shutdown
        # when a filesystem or an inherited error sink is stuck.
        self.thread.join(timeout=2)


def run(directory):
    root = Path(directory).resolve()
    settings = json.loads((root / 'settings.json').read_text(encoding='utf-8'))
    logs = root / 'request-logs'
    runtime_directory = root / 'runtime-logs'
    env = os.environ.copy()
    env.pop('CONFIG_URL', None)
    env.pop('MIORA_ADMIN_PASSWORD', None)
    env.update({
        'PORT': str(settings['port']),
        'MIORA_ADMIN_USERNAME': settings.get('username', 'admin'),
        'MIORA_ADMIN_PASSWORD_HASH': settings['password_hash'],
        'MIORA_ADMIN_ORIGIN': f"http://127.0.0.1:{settings['port']}",
        'MIORA_ADMIN_SECURE_COOKIE': 'false',
        'MIORA_BOOTSTRAP_EMPTY_CONFIG': 'true',
        'UNI_API_CONFIG_PATH': str(root / 'api.yaml'),
        'RUST_RESPONSES_CONFIG_SNAPSHOT_PATH': str(root / 'snapshot.json'),
        'UNI_API_SHARED_MEMORY_RESERVATION_PATH': str(root / 'memory-ledger'),
        'RUST_REQUEST_SPOOL_DIRECTORY': str(root / 'spool'),
        'MIORA_PLUGIN_STORE_DIR': str(root / 'plugins'),
        'MIORA_PLUGIN_ALLOW_LOOPBACK_HTTP': 'true',
        'MIORA_REQUEST_LOG_DIR': str(logs),
        'MIORA_RUNTIME_LOG_MANAGED': '1',
        'MIORA_RUNTIME_LOG_DIR': str(runtime_directory),
        'DISABLE_DATABASE': 'false',
        'DB_TYPE': 'sqlite',
        'DB_PATH': str(root / 'stats.db'),
    })
    policy = runtime_policy(logs / 'index.sqlite')
    output = RotatingOutput(runtime_directory, *(policy or (DEFAULT_FILE_BYTES, DEFAULT_RETAINED_FILES)))
    process = subprocess.Popen([str(root / 'uni-api-front.exe')], cwd=root, env=env,
                               stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT, bufsize=0, creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
    (root / 'server.pid').write_text(str(process.pid), encoding='ascii')
    (root / 'supervisor.pid').write_text(str(os.getpid()), encoding='ascii')
    print(f"Miora started: http://127.0.0.1:{settings['port']}/login (PID {process.pid})", flush=True)
    pump = RuntimeLogPump(output, logs / 'index.sqlite')
    try:
        while True:
            data = os.read(process.stdout.fileno(), 16 * 1024)
            if not data:
                break
            pump.submit(data)
    finally:
        process.stdout.close()
        pump.close()
    return process.wait()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--directory', type=Path, default=Path(__file__).resolve().parents[1] / '.miora/preview')
    raise SystemExit(run(parser.parse_args().directory))
