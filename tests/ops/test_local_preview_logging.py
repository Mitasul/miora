"""Local preview runtime log rotation; no gateway or supplier network calls."""
import importlib.util
from contextlib import closing
import json
from pathlib import Path
import sqlite3
import tempfile
import threading
import time
import unittest
from unittest import mock

MODULE_PATH = Path(__file__).resolve().parents[2] / 'scripts/local_preview.py'
spec = importlib.util.spec_from_file_location('miora_local_preview', MODULE_PATH)
preview = importlib.util.module_from_spec(spec)
spec.loader.exec_module(preview)


class RuntimeLogRotationTests(unittest.TestCase):
    def test_bounded_files_preserve_the_latest_output(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = preview.RotatingOutput(root, 64, 3)
            data = bytes(range(200))
            output.write(data)
            output.close()
            files = list(root.glob('runtime*.log'))
            self.assertLessEqual(len(files), 3)
            self.assertTrue(all(path.stat().st_size <= 64 for path in files))
            saved = b''.join(path.read_bytes() for path in
                             [root / 'runtime.2.log', root / 'runtime.1.log', root / 'runtime.log'] if path.exists())
            self.assertEqual(saved, data[-len(saved):])

    def test_changed_limits_apply_without_restarting_the_writer(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = preview.RotatingOutput(root, 64, 3)
            output.write(b'a' * 180)
            output.configure(32, 2)
            output.write(b'b' * 70)
            output.close()
            files = list(root.glob('runtime*.log'))
            self.assertLessEqual(len(files), 2)
            self.assertTrue(all(path.stat().st_size <= 32 for path in files))
            self.assertEqual((root / 'runtime.log').read_bytes(), b'b' * 6)

    def test_one_file_means_no_archive(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = preview.RotatingOutput(root, 10, 1)
            output.write(b'0123456789remaining')
            output.close()
            self.assertEqual([path.name for path in root.glob('runtime*.log')], ['runtime.log'])
            self.assertEqual((root / 'runtime.log').read_bytes(), b'remaining')

    def test_writer_and_error_sink_failure_do_not_stop_the_pump(self):
        class BrokenOutput:
            def __init__(self):
                self.called = threading.Event()
                self.closed = False

            def configure(self, *_):
                pass

            def write(self, _):
                self.called.set()
                raise OSError('fixture disk full')

            def close(self):
                self.closed = True

        class BrokenErrorSink:
            def write(self, _):
                raise OSError('fixture stderr full')

            def flush(self):
                raise OSError('fixture stderr full')

        with tempfile.TemporaryDirectory() as directory, mock.patch('sys.stderr', BrokenErrorSink()):
            output = BrokenOutput()
            pump = preview.RuntimeLogPump(output, Path(directory) / 'not-created.sqlite')
            pump.submit(b'first event')
            self.assertTrue(output.called.wait(1))
            for _ in range(200):
                pump.submit(b'more gateway output')
            pump.close()
            self.assertFalse(pump.thread.is_alive())
            self.assertGreaterEqual(pump.write_failures, 1)
            self.assertTrue(output.closed)

    def test_invalid_rotation_size_does_not_create_a_zero_progress_writer(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(ValueError):
                preview.RotatingOutput(directory, 0, 3)

    def test_quiet_gateway_still_applies_runtime_limit_changes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / 'index.sqlite'
            with closing(sqlite3.connect(path)) as connection, connection:
                connection.execute('CREATE TABLE log_settings(id INTEGER PRIMARY KEY, revision INTEGER, config_json TEXT)')
                connection.execute('INSERT INTO log_settings VALUES(1,1,?)',
                                   (json.dumps({'runtime': {'file_max_bytes': 64, 'retained_files': 2}}),))
            configured = threading.Event()

            class ObservedOutput(preview.RotatingOutput):
                def configure(self, *values):
                    super().configure(*values)
                    configured.set()

            output = ObservedOutput(root / 'runtime', 64, 2)
            output.write(b'a' * 96)
            pump = preview.RuntimeLogPump(output, path)
            try:
                self.assertTrue(configured.wait(1), 'initial policy was not read')
                # Change the saved policy after the pump has started. Do not
                # submit output: the refresh must work while the child is quiet.
                with closing(sqlite3.connect(path)) as connection, connection:
                    connection.execute('UPDATE log_settings SET revision=2, config_json=?',
                                       (json.dumps({'runtime': {'file_max_bytes': 32, 'retained_files': 1}}),))
                deadline = time.monotonic() + 3
                while output.file_max_bytes != 32 and time.monotonic() < deadline:
                    time.sleep(.02)
                self.assertEqual((output.file_max_bytes, output.retained_files), (32, 1))
            finally:
                pump.close()
            self.assertEqual(len(list((root / 'runtime').glob('runtime*.log'))), 1)

    def test_runtime_policy_reads_the_gateway_settings_table(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'index.sqlite'
            self.assertIsNone(preview.runtime_policy(path))
            with closing(sqlite3.connect(path)) as connection, connection:
                connection.execute('CREATE TABLE log_settings(id INTEGER PRIMARY KEY, revision INTEGER, config_json TEXT)')
                connection.execute('INSERT INTO log_settings VALUES(1,1,?)',
                                   (json.dumps({'runtime': {'file_max_bytes': 65536, 'retained_files': 2}}),))
            self.assertEqual(preview.runtime_policy(path), (65536, 2))
            with closing(sqlite3.connect(path)) as connection, connection:
                connection.execute('UPDATE log_settings SET revision=2, config_json=?',
                                   (json.dumps({'runtime': {'file_max_bytes': 131072, 'retained_files': 4}}),))
            self.assertEqual(preview.runtime_policy(path), (131072, 4))


if __name__ == '__main__':
    unittest.main()
