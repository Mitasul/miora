#!/usr/bin/env python3
"""HTTP regression for the password-authenticated admin boundary (stdlib only).

MIORA_ADMIN_PASSWORD is read from the environment, never printed. Run against a
throwaway instance; --exercise-writes briefly creates then deletes a test channel.
Rate-limit probing runs last because it deliberately consumes this IP's allowance.
"""
import argparse
import hashlib
import http.client
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import tempfile
import time
import sys
from urllib.parse import urlsplit


class Client:
    def __init__(self, base):
        self.base = urlsplit(base)
        if self.base.scheme not in ("http", "https") or not self.base.hostname:
            raise ValueError("base URL must be HTTP(S)")
        self.cookie = None
        self.csrf = None

    def request(self, method, path, body=None, headers=None, authenticated=True):
        cls = http.client.HTTPSConnection if self.base.scheme == "https" else http.client.HTTPConnection
        conn = cls(self.base.hostname, self.base.port, timeout=30)
        request_headers = {"Accept": "application/json", "Origin": f"{self.base.scheme}://{self.base.netloc}"}
        if body is not None:
            request_headers["Content-Type"] = "application/json"
            body = json.dumps(body).encode("utf-8")
        if authenticated and self.cookie:
            request_headers["Cookie"] = self.cookie
        request_headers.update(headers or {})
        try:
            conn.request(method, path, body=body, headers=request_headers)
            result = conn.getresponse()
            raw = result.read()
            response_headers = dict((name.lower(), value) for name, value in result.getheaders())
            try:
                data = json.loads(raw)
            except (ValueError, UnicodeError):
                data = None
            return result.status, response_headers, data, raw
        finally:
            conn.close()


def check(condition, message):
    if not condition:
        raise AssertionError(message)


def expect(client, method, path, status, **kwargs):
    result = client.request(method, path, **kwargs)
    check(result[0] == status, f"{method} {path}: expected {status}, got {result[0]}")
    check(result[1].get("cache-control") == "no-store", f"{path}: missing no-store")
    check("access-control-allow-origin" not in result[1], f"{path}: unexpected CORS")
    if status >= 400:
        check(isinstance(result[2], dict) and isinstance(result[2].get("error"), dict), f"{path}: expected structured error")
    return result


def run_isolated(binary):
    binary = Path(binary).resolve()
    check(binary.is_file(), f"Gateway binary not found: {binary}")
    with tempfile.TemporaryDirectory(prefix="miora-admin-") as directory:
        root = Path(directory)
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        base = f"http://127.0.0.1:{port}"
        password = "test-admin-" + secrets.token_hex(16)
        env = {key: value for key, value in os.environ.items() if not key.startswith(("MIORA_ADMIN_", "FACTS_S3_", "MIORA_PLUGIN_")) and key not in ("CONFIG_URL", "MIORA_TEST_USER_BEARER")}
        env.update(PORT=str(port), DISABLE_DATABASE="true", MIORA_BOOTSTRAP_EMPTY_CONFIG="true", UNI_API_CONFIG_PATH=str(root / "api.yaml"), RUST_RESPONSES_CONFIG_SNAPSHOT_PATH=str(root / "snapshot.json"), UNI_API_SHARED_MEMORY_RESERVATION_PATH=str(root / "ledger"), RUST_REQUEST_SPOOL_DIRECTORY=str(root / "spool"), RUST_REQUEST_SPOOL_DISK_RESERVE_BPS="0", RUST_REQUEST_SPOOL_INODE_RESERVE_BPS="0", MIORA_PLUGIN_STORE_DIR=str(root / "plugins"), MIORA_ADMIN_USERNAME="admin", MIORA_ADMIN_PASSWORD=password, MIORA_ADMIN_SECURE_COOKIE="false", MIORA_ADMIN_ORIGIN=base, NO_PROXY="127.0.0.1,localhost")

        def phase(name, server_env, flags):
            with (root / f"{name}.log").open("w+", encoding="utf-8") as log:
                process = subprocess.Popen([str(binary)], cwd=root, env=server_env, stdout=log, stderr=log, creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
                try:
                    ready = False
                    for _ in range(300):
                        if process.poll() is not None:
                            break
                        try:
                            if Client(base).request("GET", "/healthz", authenticated=False)[0] == 200:
                                ready = True
                                break
                        except (OSError, http.client.HTTPException):
                            pass
                        time.sleep(0.05)
                    check(ready, f"{name}: isolated gateway failed to become ready (exit={process.poll()})")
                    check((root / "api.yaml").is_file(), "bootstrap did not create an empty source configuration")
                    test_env = dict(server_env, MIORA_ADMIN_PASSWORD=password)
                    result = subprocess.run([sys.executable, str(Path(__file__).resolve()), "--base-url", base, *flags], env=test_env, timeout=120, creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
                    check(result.returncode == 0, f"{name}: HTTP regression child failed")
                finally:
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
        phase("bootstrap-and-crud", env, ["--exercise-writes", "--expect-empty"])
        disabled = dict(env, MIORA_BOOTSTRAP_EMPTY_CONFIG="false")
        disabled.pop("MIORA_ADMIN_PASSWORD", None)
        phase("disabled", disabled, ["--expect-disabled"])
        salt = secrets.token_bytes(32)
        verifier = hashlib.pbkdf2_hmac("sha256", password.encode(), salt, 600000, 32)
        hashed = dict(disabled, MIORA_ADMIN_PASSWORD_HASH=f"pbkdf2-sha256$600000${salt.hex()}${verifier.hex()}")
        phase("hash-login", hashed, ["--skip-rate-limit", "--expect-empty"])
    print("PASS: isolated empty bootstrap, plaintext/hash authentication, disabled-account fail-closed")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", nargs="?", help="Start and stop isolated test gateways using this binary")
    parser.add_argument("--base-url", default=os.getenv("MIORA_TEST_BASE_URL", "http://127.0.0.1:8080"))
    parser.add_argument("--exercise-writes", action="store_true", help="Create/update/delete a disposable channel in the configured source file")
    parser.add_argument("--skip-rate-limit", action="store_true")
    parser.add_argument("--expect-disabled", action="store_true", help="Verify absent admin credentials fail closed")
    parser.add_argument("--expect-empty", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.binary:
        run_isolated(args.binary)
        return
    client = Client(args.base_url)
    username = os.getenv("MIORA_ADMIN_USERNAME", "admin")
    password = os.getenv("MIORA_ADMIN_PASSWORD", "")
    for path in ("session", "overview", "channels", "api-keys", "logs", "settings", "plugins", "interceptors"):
        expect(client, "GET", f"/admin/api/{path}", 401, authenticated=False)
    bearer = os.getenv("MIORA_TEST_USER_BEARER", "not-a-valid-admin-even-if-name-contains-admin")
    expect(client, "GET", "/admin/api/overview", 401, headers={"Authorization": "Bearer " + bearer}, authenticated=False)
    if args.expect_disabled:
        expect(client, "POST", "/admin/api/login", 503, body={"username": username, "password": "unused-password"})
        print("PASS: admin credentials absent, login disabled, Bearer rejected")
        return
    check(bool(password), "Set MIORA_ADMIN_PASSWORD to the server account password (also when server uses PASSWORD_HASH)")
    login_body = {"username": username, "password": password}
    expect(client, "POST", "/admin/api/login", 403, body=login_body, headers={"Origin": "https://cross-origin.invalid"})
    expect(client, "POST", "/admin/api/login", 401, body={"username": username, "password": "definitely-incorrect-password"})
    login = expect(client, "POST", "/admin/api/login", 200, body=login_body)
    cookie = login[1].get("set-cookie", "")
    check("HttpOnly" in cookie and "SameSite=Strict" in cookie and "Path=/admin" in cookie, "unsafe session cookie flags")
    if client.base.scheme == "https":
        check("Secure" in cookie, "HTTPS session lacks Secure cookie")
    client.cookie = cookie.split(";", 1)[0]
    client.csrf = login[2]["csrf_token"]
    check(len(client.csrf) >= 40, "CSRF token entropy shape is too short")
    check("token" not in login[2] and "session_token" not in login[2], "session token must not be returned in JSON")
    check(login[2]["user"]["role"] == "admin", "session role mismatch")
    session = expect(client, "GET", "/admin/api/session", 200)
    check(session[2]["csrf_token"] == client.csrf, "session CSRF mismatch")
    for path in ("overview", "channels", "api-keys", "logs", "usage", "settings", "plugins", "interceptors"):
        result = expect(client, "GET", f"/admin/api/{path}", 200)
        check(password.encode() not in result[3], f"{path}: password leaked")
        if bearer != "not-a-valid-admin-even-if-name-contains-admin":
            check(bearer.encode() not in result[3], f"{path}: API key leaked")
    expect(client, "PATCH", "/admin/api/settings", 403, body={})
    expect(client, "POST", "/admin/api/logout", 403, body={})
    expect(client, "POST", "/admin/api/plugins", 403, body={})
    headers = {"X-CSRF-Token": client.csrf}
    expect(client, "PATCH", "/admin/api/settings", 403, body={}, headers={**headers, "Origin": "https://cross-origin.invalid"})
    expect(client, "PATCH", "/admin/api/settings", 400, body={}, headers=headers)
    if args.expect_empty:
        overview = expect(client, "GET", "/admin/api/overview", 200)[2]
        check(overview["api_key_count"] == 0 and overview["provider_count"] == 0, "fresh admin deployment should not synthesize keys or channels")
    channels = expect(client, "GET", "/admin/api/channels", 200)[2]
    if channels.get("writable"):
        expect(client, "DELETE", "/admin/api/channels", 409, body={"revision": "stale", "provider": "nonexistent"}, headers=headers)
    if args.exercise_writes:
        check(channels.get("writable"), "--exercise-writes requires a local writable configuration source")
        provider = "admin-regression-" + secrets.token_hex(6)
        created = False
        copied = False
        try:
            result = expect(client, "POST", "/admin/api/channels", 200, body={"revision": channels["revision"], "channel": {"provider": provider, "base_url": "https://example.invalid/v1", "engine": "gpt", "api": "test-key-should-never-be-echoed", "model": ["regression-model"]}}, headers=headers)
            created = True
            revision = result[2]["revision"]
            expect(client, "PATCH", "/admin/api/channels", 200, body={"revision": revision, "provider": provider, "patch": {"model": ["regression-model-updated"]}}, headers=headers)
            rows = expect(client, "GET", "/admin/api/channels", 200)
            check(b"test-key-should-never-be-echoed" not in rows[3], "channel secret leaked")
            check(any(row["provider"] == provider and "regression-model-updated" in row["models"] for row in rows[2]["channels"]), "channel update not reflected in runtime")
            duplicate = expect(client, "POST", "/admin/api/channels", 200, body={"revision": rows[2]["revision"], "copy_from": provider, "channel": {"provider": provider + "-copy"}}, headers=headers)
            copied = True
            check(b"test-key-should-never-be-echoed" not in duplicate[3], "copy response exposed the upstream secret")
            rows = expect(client, "GET", "/admin/api/channels", 200)
            copy = next(row for row in rows[2]["channels"] if row["provider"] == provider + "-copy")
            check(copy["api_key_count"] == 1 and copy["models"] == ["regression-model-updated"], "channel copy dropped credentials or models")
            expect(client, "POST", "/admin/api/channels", 400, body={"revision": rows[2]["revision"], "copy_from": provider, "channel": {"provider": provider + "-copy"}}, headers=headers)
            expect(client, "POST", "/admin/api/channels", 404, body={"revision": rows[2]["revision"], "copy_from": "missing-source", "channel": {"provider": provider + "-missing"}}, headers=headers)
        finally:
            if copied:
                current = expect(client, "GET", "/admin/api/channels", 200)[2]
                expect(client, "DELETE", "/admin/api/channels", 200, body={"revision": current["revision"], "provider": provider + "-copy"}, headers=headers)
            if created:
                current = expect(client, "GET", "/admin/api/channels", 200)[2]
                expect(client, "DELETE", "/admin/api/channels", 200, body={"revision": current["revision"], "provider": provider}, headers=headers)
    if args.exercise_writes:
        keys = expect(client, "GET", "/admin/api/api-keys", 200)[2]
        created_id = None
        token = None
        try:
            created = expect(client, "POST", "/admin/api/api-keys", 200, body={"revision": keys["revision"], "models": ["all"]}, headers=headers)[2]
            created_id = created["id"]
            token = created["api_key"]
            check(created["show_once"] is True and token.startswith("sk-") and len(token) >= 43, "new key entropy / one-time reveal contract")
            expect(client, "GET", "/admin/api/overview", 401, authenticated=False, headers={"Authorization": "Bearer " + token})
            rows = expect(client, "GET", "/admin/api/api-keys", 200)
            check(token.encode() not in rows[3], "new key leaked through list")
            row = next(row for row in rows[2]["api_keys"] if row["id"] == created_id)
            check(row["role"] == "user", "new key was implicitly promoted")
            expect(client, "PATCH", "/admin/api/api-keys", 400, body={"revision": rows[2]["revision"], "id": created_id, "models": []}, headers=headers)
            expect(client, "PATCH", "/admin/api/api-keys", 400, body={"revision": rows[2]["revision"], "id": created_id, "role": "superadmin"}, headers=headers)
            expect(client, "PATCH", "/admin/api/api-keys", 200, body={"revision": rows[2]["revision"], "id": created_id, "models": ["regression-model"]}, headers=headers)
        finally:
            if created_id:
                current = expect(client, "GET", "/admin/api/api-keys", 200)[2]
                expect(client, "DELETE", "/admin/api/api-keys", 200, body={"revision": current["revision"], "id": created_id}, headers=headers)
                revoked = client.request("GET", "/v1/models", authenticated=False, headers={"Authorization": "Bearer " + token})
                check(revoked[0] in (401, 403), "deleted API key still authorizes the gateway")
    old_cookie = client.cookie
    old_csrf = client.csrf
    rotated = expect(client, "POST", "/admin/api/login", 200, body=login_body)
    client.cookie = rotated[1]["set-cookie"].split(";", 1)[0]
    client.csrf = rotated[2]["csrf_token"]
    check(client.cookie != old_cookie, "login did not rotate session")
    expect(client, "GET", "/admin/api/session", 401, headers={"Cookie": old_cookie})
    expect(client, "POST", "/admin/api/logout", 403, body={}, headers={"X-CSRF-Token": old_csrf})
    logout = expect(client, "POST", "/admin/api/logout", 200, body={}, headers={"X-CSRF-Token": client.csrf})
    check("Max-Age=0" in logout[1].get("set-cookie", ""), "logout did not expire cookie")
    expect(client, "GET", "/admin/api/session", 401)
    if not args.skip_rate_limit:
        limited = False
        for _ in range(10):
            result = client.request("POST", "/admin/api/login", body={"username": username, "password": "incorrect-password"}, authenticated=False)
            if result[0] == 429:
                check(result[1].get("retry-after") == "60", "rate limit should expose Retry-After")
                limited = True
                break
            check(result[0] == 401, "unexpected rate-limit probe response")
        check(limited, "login rate limit never applied")
    print("PASS: admin authentication, cookie/CSRF/origin controls, permission boundary, read APIs, rotation/logout" + (", persistent channel and API-key CRUD" if args.exercise_writes else "") + (", rate limit" if not args.skip_rate_limit else ""))


if __name__ == "__main__":
    try:
        main()
    except (AssertionError, KeyError, ValueError, OSError, http.client.HTTPException, subprocess.SubprocessError) as exc:
        print(f"FAIL: {exc}", file=sys.stderr)
        sys.exit(1)
