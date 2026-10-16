"""Real admin settings persistence, restart, rollback and remote-source regression."""
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import sys
import tempfile
import threading
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import yaml

from http_fixture_process import wait_for_gateway, stop_gateway, report_gateway_failure
from verify_admin import Client, check, expect


SCOPE = "__uni_console_global_preferences__"


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def mutation(view, values, **options):
    return {"revision": view["revision"], "operation_id": secrets.token_hex(16), "changes": [
        {"provider": SCOPE, "set": {f"/preferences/{key}": value for key, value in values.items()},
         "remove": [], "reset": False, **options}
    ]}


@contextmanager
def gateway(binary, root, env, name):
    with (root / f"{name}.log").open("w", encoding="utf-8") as log:
        process = subprocess.Popen([str(binary)], cwd=root, env=env, stdout=log, stderr=log, creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
        try:
            wait_for_gateway(process, int(env["PORT"]))
            client = Client(env["MIORA_ADMIN_ORIGIN"])
            login = expect(client, "POST", "/admin/api/login", 200, body={
                "username": "admin", "password": env["MIORA_ADMIN_PASSWORD"]})
            client.cookie = login[1]["set-cookie"].split(";", 1)[0]
            client.csrf = login[2]["csrf_token"]
            yield client, {"X-CSRF-Token": client.csrf}, process
        except Exception:
            report_gateway_failure(process, log, root / f"{name}.log")
            raise
        finally:
            stop_gateway(process)


def environment(root, disabled):
    env = {key: value for key, value in os.environ.items()
           if not key.startswith(("MIORA_ADMIN_", "MIORA_PLUGIN_", "FACTS_S3_", "UNI_API_CONTROL_RESTORE"))
           and key not in ("CONFIG_URL", "DATABASE_URL", "MIORA_TEST_USER_BEARER")}
    port = free_port()
    env.update(PORT=str(port), DISABLE_DATABASE=disabled, DB_PATH=str(root / "stats.db"), MIORA_BOOTSTRAP_EMPTY_CONFIG="false",
               UNI_API_CONFIG_PATH=str(root / "api.yaml"),
               RUST_RESPONSES_CONFIG_SNAPSHOT_PATH=str(root / "snapshot.json"),
               UNI_API_SHARED_MEMORY_RESERVATION_PATH=str(root / "ledger"),
               RUST_REQUEST_SPOOL_DIRECTORY=str(root / "spool"),
               RUST_REQUEST_SPOOL_DISK_RESERVE_BPS="0", RUST_REQUEST_SPOOL_INODE_RESERVE_BPS="0",
               MIORA_PLUGIN_STORE_DIR=str(root / "plugins"), MIORA_ADMIN_USERNAME="admin",
               MIORA_ADMIN_PASSWORD="SettingsRegression-" + secrets.token_hex(20),
               MIORA_ADMIN_SECURE_COOKIE="false", MIORA_ADMIN_ORIGIN=f"http://127.0.0.1:{port}",
               NO_PROXY="127.0.0.1,localhost")
    return env


def verify_local(binary, disabled):
    with tempfile.TemporaryDirectory(prefix="miora-settings-") as directory:
        root = Path(directory)
        config = {"providers": [{"provider": "fixture", "engine": "gpt",
                                "base_url": "https://example.invalid/v1", "api": "${UPSTREAM_SECRET}", "model": ["fixture-model"]}],
                  "api_keys": [{"api": "legacy-test-key", "role": "admin", "model": ["all"]}],
                  "preferences": {"model_timeout": 100, "custom_option": {"credential": "preserve-secret"}},
                  "vendor_extension": {"preserve": True}}
        source = root / "api.yaml"
        source.write_text(json.dumps(config), encoding="utf-8")
        env = environment(root, disabled)
        with gateway(binary, root, env, "save") as (client, headers, process):
            legacy_headers = {"Authorization": "Bearer legacy-test-key"}
            legacy_result = client.request("GET", f"/v1/channel-settings?provider={SCOPE}", headers=legacy_headers, authenticated=False)
            check(legacy_result[0] == 200, "legacy settings view failed")
            legacy = legacy_result[2]
            overlay = mutation(legacy, {"model_timeout": 81, "keepalive_interval": 4})
            check(client.request("PATCH", "/v1/channel-settings", overlay,
                                 headers=legacy_headers, authenticated=False)[0] == 200, "legacy override failed")
            view = expect(client, "GET", "/admin/api/settings", 200)[2]
            check(view["writable"] and view["persistence"] == "source_file", "settings are not persistent")
            check(view["resolved_fields"]["/preferences/model_timeout"]["value"] == 81, "legacy override missing")
            values = {"model_timeout": 62, "cooldown_period": 3, "api_key_cooldown_period": 7,
                      "api_key_rate_limit_cooldown_period": 91, "api_key_quota_cooldown_period": 121,
                      "timeout_policy": {"default": {"total": 75}, "rules": []},
                      "hedging": {"enabled": True, "max_inflight_attempts": 2, "winner_policy": "first_valid_success"}}
            payload = mutation(view, values)
            original = source.read_bytes()
            expect(client, "POST", "/admin/api/settings/validate", 200, body=payload, headers=headers)
            check(source.read_bytes() == original, "validation wrote configuration")
            check(expect(client, "GET", "/admin/api/settings", 200)[2] == view, "validation changed runtime")
            for invalid in [mutation(view, {"model_timeout": -1}), mutation(view, {"hedging": {"max_inflight_attempts": 5}}),
                            mutation(view, {"unknown": 1}), mutation(view, {"timeout_policy": {"default": {"total": "secret-value"}}}),
                            mutation(view, {"model_timeout": 20}, copy_to_key="legacy-test-key")]:
                result = expect(client, "PATCH", "/admin/api/settings", 400, body=invalid, headers=headers)
                check(b"secret-value" not in result[3], "validation error leaked input")
            check(source.read_bytes() == original, "invalid operation changed source")
            mixed = mutation(view, {"model_timeout": 22})
            mixed["changes"].append({"provider": "fixture", "set": {}, "remove": []})
            expect(client, "PATCH", "/admin/api/settings", 400, body=mixed, headers=headers)
            saved = expect(client, "PATCH", "/admin/api/settings", 200, body=payload, headers=headers)[2]
            check(saved["persistence"] == "source_file", "save reports nonpersistent storage")
            check(expect(client, "PATCH", "/admin/api/settings", 200, body=payload, headers=headers)[2] == saved,
                  "same operation is not idempotent")
            reused = mutation(view, {"model_timeout": 45})
            reused["operation_id"] = payload["operation_id"]
            expect(client, "PATCH", "/admin/api/settings", 409, body=reused, headers=headers)
            expect(client, "PATCH", "/admin/api/settings", 409,
                   body=mutation(view, {"model_timeout": 46}), headers=headers)
            stored = yaml.safe_load(source.read_text(encoding="utf-8"))
            for key, value in {**values, "keepalive_interval": 4}.items():
                check(stored["preferences"][key] == value, f"{key} not written to source")
            for key in ("providers", "api_keys", "vendor_extension"):
                check(stored[key] == config[key], f"unrelated {key} changed")
            check(stored["preferences"]["custom_option"] == config["preferences"]["custom_option"], "custom preference lost")
            legacy_result = client.request("GET", f"/v1/channel-settings?provider={SCOPE}", headers=legacy_headers, authenticated=False)
            check(legacy_result[0] == 200, "legacy settings view failed after save")
            legacy = legacy_result[2]
            check(not legacy["override_paths"] and legacy["base"]["preferences"]["model_timeout"] == 62,
                  "persistent settings still depend on a runtime overlay")
            current = expect(client, "GET", "/admin/api/settings", 200)[2]
            original = source.read_bytes()
            snapshot = root / "snapshot.json"
            snapshot_bytes = snapshot.read_bytes()
            source_blocker = root / f".api.yaml.{process.pid}.tmp"
            source_blocker.mkdir()
            try:
                expect(client, "PATCH", "/admin/api/settings", 400,
                       body=mutation(current, {"model_timeout": 49}), headers=headers)
                check(source.read_bytes() == original, "failed source write changed source")
                check(snapshot.read_bytes() == snapshot_bytes, "failed source write changed published snapshot")
                check(expect(client, "GET", "/admin/api/settings", 200)[2] == current, "failed source write changed settings")
            finally:
                source_blocker.rmdir()
            backup = root / "snapshot.backup.json"
            snapshot.rename(backup)
            snapshot.mkdir()
            try:
                expect(client, "PATCH", "/admin/api/settings", 400,
                       body=mutation(current, {"model_timeout": 47}), headers=headers)
                check(source.read_bytes() == original, "publication failure did not roll back source")
                rolled_back = expect(client, "GET", "/admin/api/settings", 200)[2]
                check({key: value for key, value in rolled_back.items() if key != "revision"} ==
                      {key: value for key, value in current.items() if key != "revision"}, "publication failure changed settings")
            finally:
                snapshot.rmdir()
                backup.rename(snapshot)
            external = dict(stored)
            external["vendor_extension"] = {"external_edit": True}
            source.write_text(json.dumps(external), encoding="utf-8")
            expect(client, "PATCH", "/admin/api/settings", 409,
                   body=mutation(current, {"model_timeout": 48}), headers=headers)
            current = expect(client, "GET", "/admin/api/settings", 200)[2]
            removal = mutation(current, {})
            removal["changes"][0]["remove"] = ["/preferences/keepalive_interval"]
            expect(client, "PATCH", "/admin/api/settings", 200, body=removal, headers=headers)
            persisted = yaml.safe_load(source.read_text(encoding="utf-8"))
            check("keepalive_interval" not in persisted["preferences"], "removed preference still persisted")
        with gateway(binary, root, env, "restart") as (client, headers, process):
            view = expect(client, "GET", "/admin/api/settings", 200)[2]
            for key, value in values.items():
                check(view["effective"]["preferences"][key] == value, f"{key} lost after process restart")
            check(view["resolved_fields"]["/preferences/keepalive_interval"]["source"] == "default", "removal did not survive restart")
            check(yaml.safe_load(source.read_text(encoding="utf-8")) == persisted, "restart rewrote persisted configuration")
        print(f"PASS: real save/restart, validation, idempotency, conflicts, secret preservation, rollback (DISABLE_DATABASE={disabled})")


class RemoteConfig(BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_GET(self):
        body = b'{"providers":[],"api_keys":[],"preferences":{"model_timeout":53}}'
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def verify_remote(binary):
    with tempfile.TemporaryDirectory(prefix="miora-settings-remote-") as directory:
        root = Path(directory)
        server = ThreadingHTTPServer(("127.0.0.1", 0), RemoteConfig)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            env = environment(root, "true")
            env["CONFIG_URL"] = f"http://127.0.0.1:{server.server_port}/config"
            with gateway(binary, root, env, "remote") as (client, headers, process):
                view = expect(client, "GET", "/admin/api/settings", 200)[2]
                check(not view["writable"] and view["persistence"] == "read_only", "remote source looks writable")
                check(view["resolved_fields"]["/preferences/model_timeout"]["value"] == 53, "remote settings not loaded")
                payload = mutation(view, {"model_timeout": 54})
                for method, path in [("POST", "/admin/api/settings/validate"), ("PATCH", "/admin/api/settings")]:
                    expect(client, method, path, 403, body=payload, headers=headers)
                check(expect(client, "GET", "/admin/api/settings", 200)[2] == view, "remote settings were mutated")
                check(not (root / "api.yaml").exists(), "remote source was silently replaced by local configuration")
            print("PASS: remote CONFIG_URL stays read-only; no pretend save")
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)


if __name__ == "__main__":
    binary = Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/uni-api-front.exe").resolve()
    check(binary.is_file(), f"Gateway binary not found: {binary}")
    for disabled in ("true", "false"):
        verify_local(binary, disabled)
    verify_remote(binary)
