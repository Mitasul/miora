#!/usr/bin/env python3
"""Independent Miora channel service: Python 3.10+ stdlib, no paid upstream.
Run: python server.py --port 8765 --token local-demo-service-token
The service credential is independent from gateway caller/admin credentials.
"""
import argparse
import hmac
import json
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

MANIFEST = json.loads(Path(__file__).with_name("manifest.json").read_text(encoding="utf-8"))
MAX_BODY = 1024 * 1024


def json_bytes(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode("utf-8")


def event(value, name=None):
    prefix = ("event: " + name + "\n") if name else ""
    data = value if isinstance(value, str) else json_bytes(value).decode("utf-8")
    return (prefix + "data: " + data + "\n\n").encode("utf-8")


def text_input(body):
    if isinstance(body.get("messages"), list):
        for message in reversed(body["messages"]):
            if message.get("role") == "user":
                content = message.get("content", "")
                if isinstance(content, str):
                    return content
                if isinstance(content, list):
                    return " ".join(p.get("text", "") for p in content if isinstance(p, dict))
    value = body.get("input", "hello")
    if isinstance(value, str):
        return value
    if isinstance(value, list):
        return " ".join(text_input({"messages": [m]}) for m in value if isinstance(m, dict))
    return "hello"


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):
        # Never log headers, request envelopes, secrets, or prompt content.
        print(json.dumps({"event": "demo_plugin_http", "method": self.command,
                          "path": self.path.split("?")[0], "peer": self.client_address[0]}), flush=True)

    def authenticated(self):
        expected = "Bearer " + self.server.token
        if not hmac.compare_digest(self.headers.get("Authorization", ""), expected):
            self.reply(401, {"error": {"message": "Invalid plugin service credential"}})
            return False
        return True

    def reply(self, status, value):
        body = json_bytes(value)
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(body)
        self.close_connection = True

    def do_GET(self):
        if not self.authenticated():
            return
        if self.path == "/health":
            self.reply(200, {"ok": True, "api_version": 1, "plugin_id": MANIFEST["id"], "version": self.headers.get("x-miora-plugin-version", MANIFEST["version"])})
        elif self.path == "/manifest":
            self.reply(200, MANIFEST)
        else:
            self.reply(404, {"error": {"message": "Not found"}})

    def do_POST(self):
        if not self.authenticated():
            return
        try:
            length = int(self.headers.get("Content-Length", "0"))
            if not 0 < length <= MAX_BODY:
                raise ValueError("body size")
            self.connection.settimeout(10)
            envelope = json.loads(self.rfile.read(length))
            if envelope.get("api_version") != 1 or envelope.get("plugin", {}).get("id") != "echo":
                raise ValueError("contract version")
            config = envelope.get("channel", {}).get("config", {})
            prefix = config.get("prefix", "Echo")
            delay_ms = config.get("delay_ms", 30)
            fail_status = config.get("fail_status", 0)
            if not isinstance(prefix, str) or not isinstance(delay_ms, int) or not 0 <= delay_ms <= 3000:
                raise ValueError("config")
            if fail_status not in (0, 429, 500, 503):
                raise ValueError("fail_status")
            if self.path == "/models":
                self.reply(200, {"models": MANIFEST["models"]})
                return
            if self.path != "/execute":
                self.reply(404, {"error": {"message": "Not found"}})
                return
            request = envelope["request"]
            endpoint = request["endpoint"]
            if endpoint not in MANIFEST["capabilities"]["endpoints"]:
                self.reply(400, {"error": {"message": "Unsupported endpoint"}})
                return
            if fail_status:
                self.reply(fail_status, {"error": {"message": "Demo failure for retry testing"}})
                return
            body = request["body"]
            model = request["model"]
            text = prefix + ": " + text_input(body)
            revision = envelope["plugin"]["revision"]
            if endpoint == "/v1/embeddings":
                self.reply(200, {"object": "list", "model": model, "data": [
                    {"object": "embedding", "index": 0, "embedding": [0.1, 0.2, 0.3]}],
                    "usage": {"prompt_tokens": 1, "total_tokens": 1}})
            elif endpoint == "/v1/responses":
                self.responses_output(request, text, model, revision, delay_ms)
            else:
                self.chat(request, text, model, revision, delay_ms)
        except (ValueError, TypeError, KeyError, json.JSONDecodeError):
            self.reply(400, {"error": {"message": "Invalid plugin v1 envelope"}})
        except (BrokenPipeError, ConnectionResetError, TimeoutError):
            self.close_connection = True

    def stream(self, events, revision, delay_ms):
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Content-Length", str(sum(map(len, events))))
        self.send_header("X-Demo-Revision", str(revision))
        if self.headers.get("x-demo"):
            self.send_header("X-Demo-Interceptor", self.headers["x-demo"])
        self.send_header("Connection", "close")
        self.end_headers()
        for chunk in events:
            self.wfile.write(chunk)
            self.wfile.flush()
            if delay_ms:
                time.sleep(delay_ms / 1000)
        self.close_connection = True

    def chat(self, request, text, model, revision, delay_ms):
        base = {"id": "chatcmpl-echo-" + request["id"], "created": int(time.time()), "model": model}
        usage = {"prompt_tokens": 1, "completion_tokens": len(text.split()), "total_tokens": 1 + len(text.split())}
        if not request.get("stream"):
            self.reply(200, {**base, "object": "chat.completion", "choices": [
                {"index": 0, "message": {"role": "assistant", "content": text}, "finish_reason": "stop"}], "usage": usage})
            return
        def chunk(delta, reason=None):
            return {**base, "object": "chat.completion.chunk", "choices": [{"index": 0, "delta": delta, "finish_reason": reason}]}
        events = [event(chunk({"role": "assistant"}))]
        for word in text.split(" "):
            events.append(event(chunk({"content": word + " "})))
        events.extend([event(chunk({}, "stop")), event({**base, "object": "chat.completion.chunk", "choices": [], "usage": usage}), event("[DONE]")])
        self.stream(events, revision, delay_ms)

    def responses_output(self, request, text, model, revision, delay_ms):
        part = {"type": "output_text", "text": text, "annotations": []}
        item = {"id": "msg_echo", "type": "message", "role": "assistant", "status": "completed", "content": [part]}
        response = {"id": "resp_echo_" + request["id"], "object": "response", "created_at": int(time.time()), "model": model,
                    "status": "completed", "output": [item], "usage": {"input_tokens": 1, "output_tokens": len(text.split()), "total_tokens": 1 + len(text.split())}}
        if not request.get("stream"):
            self.reply(200, response)
            return
        values = [
            ("response.created", {"response": {**response, "status": "in_progress", "output": []}}),
            ("response.output_item.added", {"output_index": 0, "item": {**item, "status": "in_progress", "content": []}}),
            ("response.content_part.added", {"output_index": 0, "content_index": 0, "item_id": item["id"], "part": {**part, "text": ""}}),
        ]
        for word in text.split(" "):
            values.append(("response.output_text.delta", {"output_index": 0, "content_index": 0, "item_id": item["id"], "delta": word + " "}))
        values.extend([
            ("response.output_text.done", {"output_index": 0, "content_index": 0, "item_id": item["id"], "text": text}),
            ("response.content_part.done", {"output_index": 0, "content_index": 0, "item_id": item["id"], "part": part}),
            ("response.output_item.done", {"output_index": 0, "item": item}),
            ("response.completed", {"response": response}),
        ])
        self.stream([event({"type": name, "sequence_number": index, **value}, name) for index, (name, value) in enumerate(values)], revision, delay_ms)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument("--token", default="local-demo-service-token", help="Independent service bearer credential; change outside a local demo")
    args = parser.parse_args()
    server = ThreadingHTTPServer((args.host, args.port), Handler)
    server.token = args.token
    print(json.dumps({"event": "demo_plugin_ready", "host": args.host, "port": server.server_port}), flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
