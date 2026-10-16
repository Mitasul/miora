#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Independent, real HTTP Chat forwarding sample (Python 3.10+ stdlib).
Not a fake model: all models/completions/SSE payloads come from the upstream.
"""
import argparse
import hmac
import http.client
import json
import os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from transport import Cancelled, Upstream

PLUGIN_ID = "header-chat"
MAX_ENVELOPE = 4 * 1024 * 1024
MAX_MODELS = 1024 * 1024
MANIFEST = json.loads(Path(__file__).with_name("manifest.json").read_text(encoding="utf-8"))


def encode(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode("utf-8")


class BadGateway(Exception):
    pass


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def setup(self):
        super().setup()
        self.connection.settimeout(10)
        self.response_started = False

    def log_message(self, *_):
        # No request envelope, prompt, URL, credential, or exception text in logs.
        pass

    def reply(self, status, value):
        self.response_started = True
        body = encode(value)
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)
        self.close_connection = True

    def authenticated(self):
        supplied = self.headers.get_all("Authorization", [])
        expected = ("Bearer " + self.server.service_token).encode("utf-8")
        if len(supplied) != 1 or not hmac.compare_digest(supplied[0].encode("utf-8"), expected):
            self.reply(401, {"error": {"message": "Invalid service credential"}})
            return False
        return True

    def do_GET(self):
        if not self.authenticated():
            return
        if self.path == "/health":
            # Actual process identity: never reflect x-miora-plugin-version.
            self.reply(200, {"ok": True, "api_version": 1, "plugin_id": PLUGIN_ID,
                             "version": self.server.plugin_version})
        elif self.path == "/manifest":
            self.reply(200, {**MANIFEST, "version": self.server.plugin_version})
        else:
            self.reply(404, {"error": {"message": "Unknown service endpoint"}})

    def envelope(self):
        lengths = self.headers.get_all("Content-Length", [])
        if len(lengths) != 1 or self.headers.get("Transfer-Encoding"):
            raise ValueError("A single Content-Length is required")
        length = int(lengths[0])
        if not 0 < length <= MAX_ENVELOPE:
            raise ValueError("Envelope too large")
        raw = self.rfile.read(length)
        if len(raw) != length:
            raise ValueError("Truncated envelope")
        value = json.loads(raw)
        if not isinstance(value, dict) or not isinstance(value.get("plugin"), dict):
            raise ValueError("Invalid envelope object")
        plugin = value["plugin"]
        if (value.get("api_version") != 1 or plugin.get("id") != PLUGIN_ID
                or plugin.get("version") != self.server.plugin_version):
            raise ValueError("Plugin protocol or actual process version mismatch")
        channel = value.get("channel")
        if not isinstance(channel, dict) or not isinstance(channel.get("config"), dict):
            raise ValueError("Invalid channel config object")
        config = channel["config"]
        base = config.get("base_url")
        key = config.get("api_key")
        timeout = config.get("timeout_seconds", 120)
        if not isinstance(base, str) or not isinstance(key, str) or not key or key == "***":
            raise ValueError("Missing upstream configuration")
        if any(c in key for c in "\r\n") or not key.isascii():
            raise ValueError("Invalid upstream credential format")
        if type(timeout) is not int or not 1 <= timeout <= 600:
            raise ValueError("timeout_seconds must be 1-600")
        self.connection.settimeout(timeout)
        return value, base, key, timeout

    def do_POST(self):
        if not self.authenticated():
            return
        self.close_connection = True
        exchange = None
        try:
            if self.path not in ("/models", "/execute"):
                self.reply(404, {"error": {"message": "Unknown service endpoint"}})
                return
            envelope, base, key, timeout = self.envelope()
            # Fresh headers only. No service bearer, admin cookie, gateway caller
            # authorization, or arbitrary envelope fields are forwarded.
            headers = {"X-Sample-Api-Key": key, "Accept-Encoding": "identity",
                       "Accept": "application/json", "Connection": "close",
                       "User-Agent": "miora-header-chat/" + self.server.plugin_version}
            if self.path == "/models":
                method, suffix, body = "GET", "/models", None
            else:
                request = envelope.get("request", {})
                if not isinstance(request, dict):
                    raise ValueError("Invalid execution request object")
                body = request.get("body")
                if (request.get("method") != "POST" or request.get("endpoint") != "/v1/chat/completions"
                        or not isinstance(body, dict) or not isinstance(request.get("model"), str)
                        or body.get("model") != request["model"]):
                    raise ValueError("Unsupported execution request")
                method, suffix = "POST", "/chat/completions"
                headers["Content-Type"] = "application/json"
                if request.get("stream"):
                    headers["Accept"] = "text/event-stream"
                body = encode(body)
            with Upstream(self.connection, base, timeout) as exchange:
                response = exchange.request(method, suffix, body, headers)
                if self.path == "/models" and 200 <= response.status < 300:
                    try:
                        catalog = json.loads(exchange.read_bounded(MAX_MODELS))
                        data = catalog["data"]
                        if not isinstance(data, list) or len(data) > 4096:
                            raise ValueError("models shape")
                        models = []
                        for model in data:
                            identifier = model["id"]
                            name = model.get("name", identifier)
                            if not isinstance(identifier, str) or not identifier or not isinstance(name, str):
                                raise ValueError("model shape")
                            models.append({"id": identifier, "name": name})
                    except (ValueError, TypeError, KeyError) as exc:
                        raise BadGateway() from exc
                    self.reply(response.status, {"models": models})
                else:
                    self.relay(exchange)
        except (Cancelled, BrokenPipeError, ConnectionResetError):
            pass
        except (ValueError, TypeError, KeyError, UnicodeError):
            if not self.response_started:
                self.reply(400, {"error": {"message": "Invalid v1 request or upstream configuration"}})
        except (OSError, http.client.HTTPException, BadGateway):
            # Once streaming has started, close without the terminating HTTP
            # chunk. Never turn truncation into a complete stream or invent DONE.
            if not self.response_started and not (exchange and exchange.cancelled.is_set()):
                self.reply(502, {"error": {"message": "Upstream transport or model discovery failed"}})

    def relay(self, exchange):
        response = exchange.response
        self.response_started = True
        self.send_response(response.status)
        for name in ("Content-Type", "Content-Encoding", "Retry-After", "X-Request-ID"):
            value = response.getheader(name)
            if value is not None and "\r" not in value and "\n" not in value:
                self.send_header(name, value)
        self.send_header("X-Header-Chat-Version", self.server.plugin_version)
        self.send_header("Connection", "close")
        no_body = response.status in (204, 304)
        self.send_header("Content-Length" if no_body else "Transfer-Encoding", "0" if no_body else "chunked")
        self.end_headers()
        if no_body:
            return
        for block in exchange.chunks():
            self.wfile.write(("%x\r\n" % len(block)).encode("ascii"))
            self.wfile.write(block)
            self.wfile.write(b"\r\n")
            self.wfile.flush()
        self.wfile.write(b"0\r\n\r\n")
        self.wfile.flush()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8766)
    parser.add_argument("--version", default="1.0.0", help="Fixed actual implementation/deployment version")
    parser.add_argument("--token", default=os.getenv("HEADER_CHAT_SERVICE_TOKEN"), help="Prefer HEADER_CHAT_SERVICE_TOKEN instead of a command-line secret")
    args = parser.parse_args()
    if not args.token or "\r" in args.token or "\n" in args.token:
        parser.error("Set a nonempty HEADER_CHAT_SERVICE_TOKEN")
    if not args.version or any(not (c.isascii() and (c.isalnum() or c in "._+-")) for c in args.version):
        parser.error("version must use ASCII letters/digits/._+-")
    server = ThreadingHTTPServer((args.host, args.port), Handler)
    server.service_token = args.token
    server.plugin_version = args.version
    print(json.dumps({"event": "header_chat_ready", "port": server.server_port, "version": args.version}), flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
