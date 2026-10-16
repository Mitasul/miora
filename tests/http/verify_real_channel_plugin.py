#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Focused offline test of a REAL forwarding plugin against an explicit fixture.
No paid service, preview configuration, production key, or external repository.
Usage: python tests/http/verify_real_channel_plugin.py PATH_TO_GATEWAY_BINARY
"""
import http.client
import json
import os
from pathlib import Path
import select
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROOT = Path(__file__).resolve().parents[2]
SAMPLE = ROOT / "examples/plugins/header-chat"
SERVICE_KEY = "fixture-only-service-bearer"
UPSTREAM_KEY = "fixture-only-upstream-header-key"
CALLER_KEY = "fixture-only-caller-key"


def encode(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode("utf-8")


def port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def sse(delta, finish=None, model="upstream-response-name"):
    return b"data: " + encode({"id": "chatcmpl-fixture", "object": "chat.completion.chunk", "model": model,
                               "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]}) + b"\n\n"


def stream_body(case, version):
    if case == "tools_stream":
        return (sse({"content": "", "tool_calls": [{"index": 0, "id": "call_fixture", "type": "function",
                      "function": {"name": "tool", "arguments": "{}"}}]})
                + sse({}, "tool_calls") + b"data: [DONE]\n\n")
    if case == "long_stream":
        return b"".join(sse({"content": f"{i}:" + "x" * 4096}) for i in range(600)) + sse({}, "stop") + b"data: [DONE]\n\n"
    start = sse({"role": "assistant", "content": "actual upstream " + version + " \u4f60\u597d"})
    if case == "half_break":
        return start + sse({"content": "partial bytes only"})
    return b": explicit mock fixture\n\n" + start + sse({"content": " more"}) + sse({}, "stop") + b"data: [DONE]\n\n"


def json_body(case, version):
    message = {"role": "assistant", "content": "actual upstream " + version + " \u4f60\u597d"}
    if case == "tools":
        message = {"role": "assistant", "content": "", "tool_calls": [
            {"id": "call_fixture", "type": "function", "function": {"name": "tool", "arguments": "{}"}}]}
    if case == "long_json":
        message["content"] = "long upstream bytes " * 100000
    value = {"id": "chatcmpl-fixture", "object": "chat.completion", "model": "upstream-response-name",
             "fixture": "explicit mock; not a paid vendor", "choices": [
                 {"index": 0, "message": message, "finish_reason": "tool_calls" if case == "tools" else "stop"}]}
    if case not in ("no_usage", "tools"):
        value["usage"] = {"prompt_tokens": 3, "completion_tokens": 4, "total_tokens": 7}
    # Distinct whitespace/UTF-8 exposes accidental JSON parsing/reserialization.
    return (json.dumps(value, ensure_ascii=False, indent=2) + "\n").encode("utf-8")


class Fixture(BaseHTTPRequestHandler):
    """This is the ONLY component allowed to generate mock completion data."""
    protocol_version = "HTTP/1.1"

    def log_message(self, *_):
        pass

    def record(self, body=None):
        case = body["messages"][-1]["content"] if body else "models"
        version = self.headers.get("User-Agent", "").removeprefix("miora-header-chat/")
        hit = {"method": self.command, "path": self.path, "body": body, "case": case, "version": version,
               "key": self.headers.get("X-Sample-Api-Key"), "authorization": self.headers.get("Authorization"),
               "cookie": self.headers.get("Cookie"), "caller_header": self.headers.get("X-Caller-Secret")}
        with self.server.lock:
            self.server.hits.append(hit)
        return case, version

    def do_GET(self):
        self.record()
        body = encode({"object": "list", "data": [{"id": "wire-chat", "name": "Fixture upstream model"}]})
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(body)
        self.close_connection = True

    def chunk(self, block):
        self.wfile.write(("%x\r\n" % len(block)).encode() + block + b"\r\n")
        self.wfile.flush()

    def disconnected(self, timeout=.05):
        ready, _, _ = select.select([self.connection], [], [], timeout)
        if not ready:
            return False
        try:
            return self.connection.recv(1, socket.MSG_PEEK) == b""
        except OSError:
            return True

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        case, version = self.record(body)
        self.close_connection = True
        is_stream = bool(body.get("stream"))
        if case == "cancel_direct_headers":
            self.server.started[case].set()
            deadline = time.monotonic() + 12
            while time.monotonic() < deadline:
                if self.disconnected():
                    self.server.cancelled[case].set()
                    break
            return  # deliberately send neither headers nor body
        fixed_length = case in ("content_length", "content_length_sse", "truncated_content_length")
        if case == "error":
            output = b'{"error":{"message":"fixture upstream refuses request","type":"quota"}}'
            self.send_response(429)
            self.send_header("Retry-After", "0")
        else:
            output = stream_body(case, version) if is_stream else json_body(case, version)
            self.send_response(200)
        self.send_header("Content-Type", "text/event-stream" if is_stream else "application/json")
        if fixed_length:
            self.send_header("Content-Length", str(len(output) + (64 if case == "truncated_content_length" else 0)))
        else:
            self.send_header("Transfer-Encoding", "chunked")
        self.send_header("Connection", "close")
        self.end_headers()
        try:
            if case.startswith("cancel_"):
                self.chunk(sse({"content": "cancel test: upstream is now silent"}))
                self.server.started[case].set()
                deadline = time.monotonic() + 12
                while time.monotonic() < deadline:
                    if self.disconnected():
                        self.server.cancelled[case].set()
                        return
                    if case == "cancel_active":
                        self.chunk(sse({"content": "tick"}))
                return
            if case == "lease":
                first = sse({"content": "leased actual upstream " + version})
                self.chunk(first)
                self.server.lease_started.set()
                if not self.server.lease_release.wait(10):
                    return
                self.chunk(sse({"content": "completed on " + version}) + sse({}, "stop") + b"data: [DONE]\n\n")
            elif case == "half_break":
                first = sse({"role": "assistant", "content": "actual upstream " + version + " \u4f60\u597d"})
                self.chunk(first)
                time.sleep(.15)  # commit first semantic data before the failure
                self.chunk(output[len(first):])
                return  # deliberately omit terminating HTTP chunk and SSE DONE
            else:
                # Real transport fragmentation includes splits inside JSON and
                # UTF-8. The plugin must preserve bytes, not parse SSE frames.
                step = 701 if is_stream and case != "long_stream" else 8191
                for offset in range(0, len(output), step):
                    if fixed_length:
                        self.wfile.write(output[offset:offset + step])
                        self.wfile.flush()
                    else:
                        self.chunk(output[offset:offset + step])
                    if case == "long_stream" and offset % (step * 16) == 0:
                        time.sleep(.002)
            if not fixed_length:
                self.wfile.write(b"0\r\n\r\n")
            self.wfile.flush()
        except (OSError, BrokenPipeError):
            if case in self.server.cancelled:
                self.server.cancelled[case].set()


def close_client(connection, response):
    if connection.sock is not None:
        try:
            connection.sock.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
    response.close()
    connection.close()


def drain(response):
    parts = []
    incomplete = False
    try:
        while True:
            part = response.read1(16384)
            if not part:
                break
            parts.append(part)
    except http.client.IncompleteRead as error:
        parts.append(error.partial)
        incomplete = True
    return b"".join(parts), incomplete


def run(binary):
    fixture = ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
    fixture.lock, fixture.hits = threading.Lock(), []
    cases = ("cancel_active", "cancel_silent", "cancel_direct_silent", "cancel_direct_headers")
    fixture.started = {case: threading.Event() for case in cases}
    fixture.cancelled = {case: threading.Event() for case in cases}
    fixture.lease_started, fixture.lease_release = threading.Event(), threading.Event()
    threading.Thread(target=fixture.serve_forever, daemon=True).start()
    processes, logs = [], []
    with tempfile.TemporaryDirectory(prefix="miora-real-plugin-") as directory:
        root = Path(directory)
        copied = root / binary.name
        shutil.copy2(binary, copied)  # do NOT hold the build output or preview binary open
        gateway_port, first_port, second_port = port(), port(), port()
        upstream_config = {"base_url": f"http://127.0.0.1:{fixture.server_port}/v1",
                           "api_key": UPSTREAM_KEY, "timeout_seconds": 120}
        configuration = {"providers": [{"provider": "real-fixture", "channel_plugin": "header-chat",
                                         "model": [{"wire-chat": "public-chat"}],
                                         "preferences": {"cooldown_period": 0}}],
                         "api_keys": [{"api": CALLER_KEY, "role": "user", "model": ["all"],
                                       "preferences": {"AUTO_RETRY": False}}]}
        (root / "api.json").write_text(json.dumps(configuration), encoding="utf-8")
        env = {k: v for k, v in os.environ.items() if not k.startswith("FACTS_S3_")
               and k not in ("CONFIG_URL", "MIORA_ADMIN_PASSWORD_HASH")}
        env.update(PORT=str(gateway_port), DISABLE_DATABASE="true", UNI_API_CONFIG_PATH=str(root / "api.json"),
                   RUST_RESPONSES_CONFIG_SNAPSHOT_PATH=str(root / "snapshot.json"),
                   UNI_API_SHARED_MEMORY_RESERVATION_PATH=str(root / "ledger"), RUST_REQUEST_SPOOL_DIRECTORY=str(root / "spool"),
                   RUST_REQUEST_SPOOL_DISK_RESERVE_BPS="0", RUST_REQUEST_SPOOL_INODE_RESERVE_BPS="0",
                   MIORA_PLUGIN_ALLOW_LOOPBACK_HTTP="true", MIORA_PLUGIN_ALLOWED_ORIGINS="", MIORA_PLUGIN_STORE_DIR=str(root / "plugins"),
                   MIORA_ADMIN_USERNAME="fixture-admin", MIORA_ADMIN_PASSWORD="fixture-password-change-me",
                   MIORA_ADMIN_SECURE_COOKIE="false", MIORA_ADMIN_ORIGIN=f"http://127.0.0.1:{gateway_port}",
                   HEADER_CHAT_SERVICE_TOKEN=SERVICE_KEY, PYTHONDONTWRITEBYTECODE="1")
        cookie, csrf = "", ""

        def launch(command, name):
            log = (root / (name + ".log")).open("w+", encoding="utf-8")
            logs.append((name, log))
            process = subprocess.Popen(command, cwd=root, env=env, stdout=log, stderr=log, creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
            processes.append(process)
            return process

        def request(target, method, path, body=None, headers=None, streaming=False):
            connection = http.client.HTTPConnection("127.0.0.1", target, timeout=15)
            all_headers = {"Content-Type": "application/json", **(headers or {})}
            connection.request(method, path, None if body is None else encode(body), all_headers)
            response = connection.getresponse()
            if streaming:
                return connection, response
            raw = response.read()
            result = response.status, {k.lower(): v for k, v in response.getheaders()}, raw
            connection.close()
            return result

        def ready(process, target, path, headers=None):
            for _ in range(160):
                assert process.poll() is None, "isolated fixture process exited"
                try:
                    if request(target, "GET", path, headers=headers)[0] == 200:
                        return
                except OSError:
                    pass
                time.sleep(.05)
            raise AssertionError("isolated fixture startup timeout")

        def admin(method, path, body=None, expected=200):
            status, _, raw = request(gateway_port, method, "/admin/api/" + path, body,
                                     {"Origin": f"http://127.0.0.1:{gateway_port}", "Cookie": cookie, "X-CSRF-Token": csrf})
            assert status == expected, (path, status, raw[:400])
            return json.loads(raw)

        def register(version, endpoint_port, expected=0, status=200):
            manifest = json.loads((SAMPLE / "manifest.json").read_text(encoding="utf-8"))
            manifest["version"] = version
            return admin("POST", "plugins", {"manifest": manifest, "endpoint": f"http://127.0.0.1:{endpoint_port}",
                         "config": upstream_config, "service_auth": {"type": "bearer", "token": SERVICE_KEY},
                         "expected_revision": expected}, expected=status)

        def payload(case, stream=False):
            return {"model": "public-chat", "messages": [{"role": "user", "content": case}], "stream": stream,
                    "temperature": 0.25, "max_tokens": 512, "fixture_extra": {"preserve": True}}

        def inference(case, stream=False):
            return request(gateway_port, "POST", "/v1/chat/completions", payload(case, stream),
                           {"Authorization": "Bearer " + CALLER_KEY, "X-Caller-Secret": "not-for-upstream"}, streaming=stream)

        def direct_envelope(case, stream=False, version="1.0.0"):
            body = payload(case, stream)
            body["model"] = "wire-chat"
            return {"api_version": 1, "plugin": {"id": "header-chat", "version": version, "revision": 1},
                    "request": {"id": "direct-test", "method": "POST", "endpoint": "/v1/chat/completions",
                                "model": "wire-chat", "requested_model": "public-chat", "stream": stream, "body": body},
                    "channel": {"name": "direct-fixture", "config": upstream_config}}

        def direct(case, stream=False, auth=SERVICE_KEY, version="1.0.0"):
            return request(first_port, "POST", "/execute", direct_envelope(case, stream, version),
                           {"Authorization": "Bearer " + auth, "Cookie": "admin-should-never-forward", "X-Caller-Secret": "private"}, streaming=stream)

        def assert_new_hit(before, case, version="1.0.0"):
            with fixture.lock:
                hits = fixture.hits[before:]
            assert len(hits) == 1, (case, "expected exactly one real upstream HTTP request", hits)
            hit = hits[0]
            assert hit["case"] == case and hit["version"] == version, hit
            assert hit["path"] == "/v1/chat/completions" and hit["method"] == "POST", hit
            expected_body = payload(case, hit["body"]["stream"])
            expected_body["model"] = "wire-chat"
            assert hit["body"] == expected_body, "plugin failed exact upstream request JSON forwarding"
            assert hit["key"] == UPSTREAM_KEY and hit["authorization"] is None and hit["cookie"] is None and hit["caller_header"] is None, hit

        try:
            first = launch([sys.executable, "-B", str(SAMPLE / "server.py"), "--port", str(first_port), "--version", "1.0.0"], "service-v1")
            second = launch([sys.executable, "-B", str(SAMPLE / "server.py"), "--port", str(second_port), "--version", "2.0.0"], "service-v2")
            for process, target in ((first, first_port), (second, second_port)):
                ready(process, target, "/health", {"Authorization": "Bearer " + SERVICE_KEY})
            _, _, raw = request(first_port, "GET", "/health", headers={"Authorization": "Bearer " + SERVICE_KEY, "X-Miora-Plugin-Version": "99.0.0"})
            assert json.loads(raw)["version"] == "1.0.0", "health reflected requested version instead of actual process version"
            before = len(fixture.hits)
            assert direct("json", auth="wrong")[0] == 401 and len(fixture.hits) == before
            assert direct("json", version="2.0.0")[0] == 400 and len(fixture.hits) == before
            gateway = launch([str(copied)], "gateway")
            ready(gateway, gateway_port, "/healthz")
            status, headers, raw = request(gateway_port, "POST", "/admin/api/login",
                                          {"username": "fixture-admin", "password": "fixture-password-change-me"},
                                          {"Origin": f"http://127.0.0.1:{gateway_port}"})
            assert status == 200, (status, raw)
            cookie, csrf = headers["set-cookie"].split(";", 1)[0], json.loads(raw)["csrf_token"]
            register("1.0.0", first_port)
            before = len(fixture.hits)
            models = admin("POST", "plugins/header-chat/models", {})
            assert models == {"models": [{"id": "wire-chat", "name": "Fixture upstream model"}]}, models
            hit = fixture.hits[before]
            assert len(fixture.hits) == before + 1 and hit["method"] == "GET" and hit["path"] == "/v1/models", hit
            assert hit["key"] == UPSTREAM_KEY and hit["authorization"] is None and hit["cookie"] is None, hit
            print("PASS actual upstream model discovery and independent service/header authentication")

            for case in ("json", "no_usage", "tools", "long_json", "content_length"):
                before = len(fixture.hits)
                status, _, raw = inference(case)
                assert status == 200 and raw == json_body(case, "1.0.0"), (case, status, raw[:300])
                assert_new_hit(before, case)
                parsed = json.loads(raw)
                assert parsed["model"] == "upstream-response-name"
                if case in ("no_usage", "tools"):
                    assert "usage" not in parsed
                if case == "tools":
                    assert parsed["choices"][0]["message"]["content"] == "" and parsed["choices"][0]["message"]["tool_calls"]
            print("PASS byte-preserving JSON, absent usage, empty text with tool_calls, aliases and long response")

            for case in ("sse", "tools_stream", "long_stream", "content_length_sse"):
                before = len(fixture.hits)
                connection, response = inference(case, True)
                assert response.status == 200
                raw, incomplete = drain(response)
                connection.close()
                assert not incomplete and raw == stream_body(case, "1.0.0"), (case, len(raw), raw[:300])
                assert b'"usage"' not in raw
                assert_new_hit(before, case)
            print("PASS raw SSE and tool deltas, arbitrary transport boundaries, >2 MiB multi-chunk stream without fabricated usage")

            before = len(fixture.hits)
            connection, response = inference("half_break", True)
            assert response.status == 200
            raw, incomplete = drain(response)
            connection.close()
            assert raw.startswith(stream_body("half_break", "1.0.0")) and b"[DONE]" not in raw, raw[-500:]
            assert_new_hit(before, "half_break")
            print("PASS halfway upstream transport failure does not fabricate DONE (incomplete HTTP:", incomplete, ")")

            before = len(fixture.hits)
            # Direct service read is streamed even for JSON, exposing an invalid
            # Content-Length as incomplete transport instead of fabricated EOF.
            envelope = direct_envelope("truncated_content_length")
            connection, response = request(first_port, "POST", "/execute", envelope,
                                           {"Authorization": "Bearer " + SERVICE_KEY}, streaming=True)
            raw, incomplete = drain(response)
            connection.close()
            assert response.status == 200 and incomplete and raw == json_body("truncated_content_length", "1.0.0")
            assert_new_hit(before, "truncated_content_length")
            before = len(fixture.hits)
            assert inference("truncated_content_length")[0] == 502
            assert_new_hit(before, "truncated_content_length")
            print("PASS Content-Length JSON/SSE and truncated Content-Length fail without false completion")

            before = len(fixture.hits)
            envelope = encode(direct_envelope("cancel_direct_headers", True))
            raw_client = socket.create_connection(("127.0.0.1", first_port), timeout=15)
            raw_client.sendall((f"POST /execute HTTP/1.1\r\nHost: 127.0.0.1:{first_port}\r\nAuthorization: Bearer {SERVICE_KEY}\r\nContent-Type: application/json\r\nContent-Length: {len(envelope)}\r\n\r\n").encode() + envelope)
            try:
                assert fixture.started["cancel_direct_headers"].wait(2)
                started = time.monotonic()
                raw_client.shutdown(socket.SHUT_RDWR)
            finally:
                raw_client.close()
            assert fixture.cancelled["cancel_direct_headers"].wait(2.5), "pre-header silent upstream was not closed"
            assert_new_hit(before, "cancel_direct_headers")
            print("PASS direct service cancellation before upstream response headers: %.3fs" % (time.monotonic() - started))

            for case in ("cancel_direct_silent", "cancel_active", "cancel_silent"):
                before = len(fixture.hits)
                connection, response = direct(case, True) if case == "cancel_direct_silent" else inference(case, True)
                assert response.status == 200
                first_line = response.readline()
                assert b"cancel test" in first_line, first_line
                assert fixture.started[case].wait(1)
                started = time.monotonic()
                close_client(connection, response)
                assert fixture.cancelled[case].wait(2.5), case + ": upstream socket not closed after client cancellation"
                assert_new_hit(before, case)
                print("PASS cancellation reaches upstream:", case, "%.3fs" % (time.monotonic() - started))

            before = len(fixture.hits)
            connection, response = inference("lease", True)
            first_line = response.readline()
            assert response.status == 200 and b"1.0.0" in first_line and fixture.lease_started.wait(1)
            register("2.0.0", second_port, 1)
            status, headers, raw = inference("json")
            assert status == 200 and raw == json_body("json", "2.0.0") and headers["x-miora-plugin-version"] == "2.0.0"
            listing = next(p for p in admin("GET", "plugins")["plugins"] if p["id"] == "header-chat")
            assert next(v for v in listing["versions"] if v["revision"] == 1)["draining"]
            fixture.lease_release.set()
            rest, incomplete = drain(response)
            connection.close()
            assert not incomplete and b"completed on 1.0.0" in rest and b"[DONE]" in rest
            with fixture.lock:
                assert [(h["case"], h["version"]) for h in fixture.hits[before:]] == [("lease", "1.0.0"), ("json", "2.0.0")]
            register("3.0.0", first_port, 2, status=400)
            listing = next(p for p in admin("GET", "plugins")["plugins"] if p["id"] == "header-chat")
            assert listing["revision"] == 2 and listing["manifest"]["version"] == "2.0.0"
            assert inference("json")[2] == json_body("json", "2.0.0")
            print("PASS independent v1/v2 processes, old stream lease, new requests on v2 and wrong-health-version rejection")

            before = len(fixture.hits)
            status, _, raw = direct("error")
            assert status == 429 and raw == b'{"error":{"message":"fixture upstream refuses request","type":"quota"}}'
            assert_new_hit(before, "error")
            before = len(fixture.hits)
            assert inference("error")[0] == 429, "gateway changed an upstream failure into success"
            assert_new_hit(before, "error", "2.0.0")
            print("PASS supplier failure status is preserved; service forwards original error bytes")
            print("PASS focused real-channel-plugin suite: all assertions passed; only local fixture services used")
        except Exception:
            for name, log in logs:
                log.flush()
                print("--- isolated", name, "---")
                print((root / (name + ".log")).read_text(encoding="utf-8", errors="replace")[-10000:])
            raise
        finally:
            fixture.lease_release.set()
            for process in reversed(processes):
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
            for _, log in logs:
                log.close()
            fixture.shutdown()
            fixture.server_close()


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    run(Path(sys.argv[1]).resolve())
