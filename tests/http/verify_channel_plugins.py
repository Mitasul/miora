#!/usr/bin/env python3
"""Offline real-gateway plugin lifecycle regression (Python stdlib only).
Usage: python tests/http/verify_channel_plugins.py target/debug/uni-api-front.exe
Creates isolated config/store, a threaded standalone plugin, and a gateway child.
"""
import copy
import http.client
import importlib.util
import json
import os
from pathlib import Path
import socket
import shutil
import subprocess
import sys
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("echo_plugin", ROOT / "examples/plugins/echo/server.py")
echo = importlib.util.module_from_spec(spec)
spec.loader.exec_module(echo)


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


class Service(echo.Handler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        self.server.hits.append((self.path, self.headers.get("x-miora-plugin-revision"),
                                 self.headers.get("x-demo"), self.headers.get("Authorization"), self.headers.get("Cookie")))
        super().do_POST()


def verify(binary):
    service = echo.ThreadingHTTPServer(("127.0.0.1", 0), Service)
    service.token = "fixture-service-secret"
    service.hits = []
    threading.Thread(target=service.serve_forever, daemon=True).start()
    with tempfile.TemporaryDirectory(prefix="miora-plugin-http-") as directory:
        root = Path(directory)
        copied_binary = root / binary.name
        shutil.copy2(binary, copied_binary)
        binary = copied_binary
        port = free_port()
        config = {"providers": [{"provider": "echo-channel", "channel_plugin": "echo", "model": ["echo-model"],
                                  "plugin_config": {"credential": "fixture-channel-secret"}, "preferences": {"cooldown_period": 0, "AUTO_RETRY": False}}],
                  "api_keys": [{"api": "fixture-caller-secret", "role": "user", "model": ["all"]}]}
        (root / "api.json").write_text(json.dumps(config), encoding="utf-8")
        env = dict(os.environ, PORT=str(port), DISABLE_DATABASE="true", UNI_API_CONFIG_PATH=str(root / "api.json"),
                   RUST_RESPONSES_CONFIG_SNAPSHOT_PATH=str(root / "snapshot.json"),
                   UNI_API_SHARED_MEMORY_RESERVATION_PATH=str(root / "ledger"), RUST_REQUEST_SPOOL_DIRECTORY=str(root / "spool"),
                   RUST_REQUEST_SPOOL_DISK_RESERVE_BPS="0", RUST_REQUEST_SPOOL_INODE_RESERVE_BPS="0",
                   MIORA_PLUGIN_ALLOW_LOOPBACK_HTTP="true", MIORA_PLUGIN_ALLOWED_ORIGINS="", MIORA_PLUGIN_STORE_DIR=str(root / "plugins"),
                   MIORA_ADMIN_USERNAME="fixture-admin", MIORA_ADMIN_PASSWORD="fixture-password-change-me", MIORA_ADMIN_SECURE_COOKIE="false",
                   MIORA_ADMIN_ORIGIN=f"http://127.0.0.1:{port}",
                   NO_PROXY="127.0.0.1,localhost")
        env = {k: v for k, v in env.items() if not k.startswith("FACTS_S3_") and k not in ("MIORA_ADMIN_PASSWORD_HASH", "CONFIG_URL")}
        cookie, csrf = "", ""
        log = (root / "gateway.log").open("w+", encoding="utf-8")
        process = None

        def request(method, path, payload=None, admin=False, stream=False):
            conn = http.client.HTTPConnection("127.0.0.1", port, timeout=15)
            headers = {"Content-Type": "application/json", "Origin": f"http://127.0.0.1:{port}"}
            if admin:
                headers.update({"Cookie": cookie, "X-CSRF-Token": csrf})
            elif path.startswith("/v1/"):
                headers["Authorization"] = "Bearer fixture-caller-secret"
            conn.request(method, path, None if payload is None else json.dumps(payload).encode(), headers)
            response = conn.getresponse()
            if stream:
                return conn, response
            raw = response.read()
            result = response.status, dict((k.lower(), v) for k, v in response.getheaders()), raw
            conn.close()
            return result

        def admin(method, path, payload=None, expected=200):
            status, headers, raw = request(method, "/admin/api/" + path, payload, admin=True)
            assert status == expected, (method, path, status, raw[:400])
            return json.loads(raw)

        def login():
            nonlocal cookie, csrf
            status, headers, raw = request("POST", "/admin/api/login", {"username": "fixture-admin", "password": "fixture-password-change-me"})
            assert status == 200, (status, raw)
            cookie = headers["set-cookie"].split(";", 1)[0]
            csrf = json.loads(raw)["csrf_token"]

        def start():
            nonlocal process
            process = subprocess.Popen([str(binary)], cwd=root, env=env, stdout=log, stderr=log, creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
            for _ in range(150):
                if process.poll() is not None:
                    raise RuntimeError("gateway exited during startup")
                try:
                    if request("GET", "/healthz")[0] == 200:
                        login()
                        return
                except OSError:
                    pass
                time.sleep(.05)
            raise RuntimeError("gateway startup timeout")

        def stop():
            if process is not None and process.poll() is None:
                process.terminate()
                try:
                    process.wait(5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()

        def upload(package, expected=0, authenticated=True, with_csrf=True, copies=1, options=None, file_bytes=None):
            boundary = "miora-http-upload-fixture"
            payload = json.dumps(package).encode() if file_bytes is None else file_bytes
            file_part = (f'--{boundary}\r\nContent-Disposition: form-data; name="file"; filename="echo.miora.json"\r\nContent-Type: application/json\r\n\r\n'.encode() + payload + b"\r\n")
            fields = {"expected_revision": expected} if options is None else options
            body = file_part * copies + (f'--{boundary}\r\nContent-Disposition: form-data; name="options"\r\n\r\n{json.dumps(fields)}\r\n--{boundary}--\r\n').encode()
            headers = {"Content-Type": f"multipart/form-data; boundary={boundary}", "Origin": f"http://127.0.0.1:{port}"}
            if authenticated:
                headers["Cookie"] = cookie
            if with_csrf:
                headers["X-CSRF-Token"] = csrf
            conn = http.client.HTTPConnection("127.0.0.1", port, timeout=15)
            conn.request("POST", "/admin/api/plugins/upload", body, headers)
            response = conn.getresponse()
            result = response.status, json.loads(response.read())
            conn.close()
            return result

        def register(expected, version, prefix, delay=80):
            manifest = copy.deepcopy(echo.MANIFEST)
            manifest["version"] = version
            body = {"manifest": manifest, "endpoint": f"http://127.0.0.1:{service.server_port}",
                    "config": {"prefix": prefix, "delay_ms": delay},
                    "service_auth": {"type": "bearer", "token": service.token}, "expected_revision": expected}
            body["format"] = "miora.plugin.v1"
            del body["expected_revision"]
            status, result = upload(body, expected)
            assert status == 200, (status, result)
            return result["plugin"]

        def inference(endpoint, streaming=False):
            payload = {"model": "echo-model", "stream": streaming}
            text = "one two three four five six seven eight nine ten eleven twelve"
            payload.update({"messages": [{"role": "user", "content": text}]} if endpoint.endswith("chat/completions") else {"input": text})
            return request("POST", endpoint, payload, stream=streaming)

        try:
            start()
            template = admin("GET", "plugins/template")
            assert template["format"] == "miora.plugin.v1"
            assert upload(template, authenticated=False)[0] == 401
            assert upload(template, with_csrf=False)[0] == 403
            assert upload(template, copies=2)[0] == 400
            assert upload(template, copies=0)[0] == 400
            assert upload(template, options={"config": {}})[0] == 400
            assert upload(template, file_bytes=b"not-json")[0] == 400
            assert upload(template, file_bytes=b" " * 262145)[0] == 413
            assert upload(template, file_bytes=b" " * 524289)[0] == 413
            assert upload(dict(template, format="foreign.plugin.v1"))[0] == 400
            for field, value in [("api_version", 2), ("id", "builtin-echo"), ("capabilities", {"endpoints": ["/unsupported"]})]:
                invalid_manifest = copy.deepcopy(template)
                invalid_manifest["manifest"][field] = value
                assert upload(invalid_manifest)[0] == 400
            duplicate_schema = copy.deepcopy(template)
            duplicate_schema["manifest"]["config_schema"] *= 2
            assert upload(duplicate_schema)[0] == 400
            untrusted = dict(template, endpoint="https://not-allowed.invalid")
            untrusted["service_auth"] = {"type": "bearer", "token": "fixture-token"}
            assert upload(untrusted)[0] == 400
            register(0, "1.0.0", "old")
            conflict = {"format": "miora.plugin.v1", "manifest": echo.MANIFEST,
                        "endpoint": f"http://127.0.0.1:{service.server_port}",
                        "service_auth": {"type": "bearer", "token": service.token}}
            assert upload(conflict)[0] == 409
            print("PASS file upload: native package, authentication, CSRF, duplicate/missing file, no configuration overrides, size limits, trust and revision conflict")
            discovered = admin("POST", "plugins/echo/models", {})
            assert discovered["models"][0]["id"] == "echo-model"
            admin("PUT", "interceptors", {"expected_revision": 0, "interceptors": [
                {"id": "demo", "enabled": True, "order": 1, "plugin_ids": ["echo"], "set_headers": {"x-demo": "ordered"}}]})
            for index, endpoint in enumerate(["/v1/chat/completions", "/v1/responses"]):
                current = next(p for p in admin("GET", "plugins")["plugins"] if p["id"] == "echo")
                if index:
                    current = register(current["revision"], "3.0.0", "old")
                executing_revision = next(v["revision"] for v in current["versions"] if v["active"])
                conn, stream = inference(endpoint, streaming=True)
                assert stream.status == 200, (endpoint, stream.status, stream.read())
                first = stream.readline()
                assert first, "no first stream frame"
                assert stream.getheader("x-miora-plugin-revision") == str(executing_revision)
                new = register(current["revision"], f"{index + 4}.0.0", "new", 0)
                status, headers, raw = inference(endpoint)
                assert status == 200, (endpoint, status, raw)
                assert b"new:" in raw and headers["x-miora-plugin-revision"] == str(new["revision"]), raw
                listing = next(p for p in admin("GET", "plugins")["plugins"] if p["id"] == "echo")
                old_version = next(v for v in listing["versions"] if v["revision"] == executing_revision)
                assert old_version["draining"] and old_version["leases"] > 0, listing
                disabled = admin("POST", "plugins/echo/disable", {"expected_revision": new["revision"]})["plugin"]
                assert inference(endpoint)[0] >= 400, "disabled plugin still accepts new request"
                tail = first + stream.read()
                conn.close()
                marker = b"[DONE]" if endpoint.endswith("chat/completions") else b"response.completed"
                assert marker in tail and b"old:" in tail, tail[-1000:]
                admin("POST", "plugins/echo/enable", {"expected_revision": disabled["revision"]})
                print("PASS real gateway lifecycle:", endpoint, "pinned old stream, new revision, disable draining")
            current = next(p for p in admin("GET", "plugins")["plugins"] if p["id"] == "echo")
            bad_manifest = copy.deepcopy(echo.MANIFEST)
            bad_manifest["api_version"] = 99
            admin("POST", "plugins", {"manifest": bad_manifest, "endpoint": f"http://127.0.0.1:{service.server_port}", "expected_revision": current["revision"]}, expected=400)
            assert inference("/v1/chat/completions")[0] == 200
            unreachable = {"manifest": copy.deepcopy(echo.MANIFEST), "endpoint": f"http://127.0.0.1:{free_port()}",
                           "config": {"prefix": "unreachable"}, "expected_revision": current["revision"],
                           "service_auth": {"type": "bearer", "token": service.token}}
            admin("POST", "plugins", unreachable, expected=400)
            retained = next(p for p in admin("GET", "plugins")["plugins"] if p["id"] == "echo")
            assert retained["revision"] == current["revision"], "failed candidate health changed active revision"
            assert inference("/v1/chat/completions")[0] == 200
            rolled = admin("POST", "plugins/echo/rollback", {"revision": 1, "expected_revision": current["revision"]})["plugin"]
            assert rolled["revision"] > current["revision"]
            status, headers, raw = inference("/v1/chat/completions")
            assert status == 200 and headers["x-miora-plugin-revision"] == "1" and b"old:" in raw
            listing = admin("GET", "plugins")
            assert "fixture-service-secret" not in json.dumps(listing)
            detail = admin("GET", "channels/detail?provider=echo-channel")
            assert "fixture-channel-secret" not in json.dumps(detail)
            assert any(hit[2] == "ordered" for hit in service.hits)
            assert all(hit[3] == "Bearer fixture-service-secret" and hit[4] is None for hit in service.hits)
            stop()
            start()
            status, headers, raw = inference("/v1/chat/completions")
            assert status == 200 and headers["x-miora-plugin-revision"] == "1"
            assert admin("GET", "interceptors")["revision"] == 1
            print("PASS model discovery, control auth isolation, header interceptors, rollback, failed validation, restart persistence and secret redaction")
        except Exception:
            log.flush()
            print((root / "gateway.log").read_text(encoding="utf-8", errors="replace")[-16000:])
            raise
        finally:
            stop()
            log.close()
            service.shutdown()
            service.server_close()


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    verify(Path(sys.argv[1]).resolve())
