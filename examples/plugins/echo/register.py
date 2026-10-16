#!/usr/bin/env python3
"""Register/update the demo through the authenticated admin API (stdlib only).
Set MIORA_ADMIN_USERNAME, MIORA_ADMIN_PASSWORD, ECHO_SERVICE_TOKEN as needed.
"""
import argparse
import http.cookiejar
import json
import os
from pathlib import Path
import urllib.request


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--gateway", default="http://127.0.0.1:8000")
    parser.add_argument("--endpoint", default="http://127.0.0.1:8765")
    parser.add_argument("--version", default="1.0.0")
    parser.add_argument("--prefix", default="Echo")
    args = parser.parse_args()
    password = os.environ.get("MIORA_ADMIN_PASSWORD")
    if not password:
        raise SystemExit("Set MIORA_ADMIN_PASSWORD; no default administrator password is used")
    base = args.gateway.rstrip("/")
    opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
    csrf = ""

    def call(method, path, body=None):
        headers = {"Content-Type": "application/json", "Origin": base}
        if csrf:
            headers["X-CSRF-Token"] = csrf
        request = urllib.request.Request(base + path, data=None if body is None else json.dumps(body).encode(), method=method, headers=headers)
        with opener.open(request, timeout=20) as response:
            return json.load(response)

    login = call("POST", "/admin/api/login", {"username": os.getenv("MIORA_ADMIN_USERNAME", "admin"), "password": password})
    csrf = login["csrf_token"]
    try:
        plugins = call("GET", "/admin/api/plugins")["plugins"]
        revision = next((p["revision"] for p in plugins if p["id"] == "echo"), 0)
        manifest = json.loads(Path(__file__).with_name("manifest.json").read_text(encoding="utf-8"))
        manifest["version"] = args.version
        result = call("POST", "/admin/api/plugins", {"manifest": manifest, "endpoint": args.endpoint,
                      "config": {"prefix": args.prefix}, "expected_revision": revision,
                      "service_auth": {"type": "bearer", "token": os.getenv("ECHO_SERVICE_TOKEN", "local-demo-service-token")}})
        print(json.dumps(result, ensure_ascii=False, indent=2))
    finally:
        call("POST", "/admin/api/logout", {})


if __name__ == "__main__":
    main()
