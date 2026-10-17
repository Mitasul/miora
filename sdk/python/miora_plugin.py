import dataclasses
import contextlib
import http.server
import json
import os
import pathlib
import secrets
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from functools import cached_property
from miora_resources import Multipart, RequestParameterError, Resource, ResourceStore, REQUEST_LIMIT, read_exact, request_parameters

SDK_VERSION = 1


@dataclasses.dataclass
class Context:
    request: dict
    config: dict
    channel: str
    authentication: dict
    cancelled: threading.Event = dataclasses.field(default_factory=threading.Event)
    resources: ResourceStore = dataclasses.field(default_factory=ResourceStore)
    request_schema: list = dataclasses.field(default_factory=list)

    @property
    def credentials(self):
        return CredentialStore(self.channel)

    @cached_property
    def parameters(self):
        return request_parameters(self.request, self.request_schema, self.resources)

    @property
    def body_resource(self):
        return self.resources.get(self.request["body_resource"])


class CredentialStore:
    def __init__(self, channel, root=None):
        if not channel:
            raise ValueError("A channel name is required for OAuth credentials")
        self.root = pathlib.Path(root or os.environ["MIORA_PLUGIN_STATE_DIR"])
        self.root.mkdir(mode=0o700, parents=True, exist_ok=True)
        self.channel = channel
        self.path = self.root / "accounts.json"

    @contextlib.contextmanager
    def transaction(self):
        lock_path = self.path.with_suffix(".lock")
        descriptor = os.open(lock_path, os.O_RDWR | os.O_CREAT, 0o600)
        with os.fdopen(descriptor, "r+b") as lock:
            if os.name == "nt":
                import msvcrt
                if os.fstat(lock.fileno()).st_size == 0:
                    lock.write(b"\0")
                    lock.flush()
                lock.seek(0)
                msvcrt.locking(lock.fileno(), msvcrt.LK_LOCK, 1)
            else:
                import fcntl
                fcntl.flock(lock.fileno(), fcntl.LOCK_EX)
            try:
                stored = self.path.read_text(encoding="utf-8") if self.path.exists() else "{}"
                accounts = json.loads(stored)
                value = accounts.get(self.channel, {})
                yield value
                if value or self.channel in accounts:
                    accounts[self.channel] = value
                encoded = json.dumps(accounts, ensure_ascii=False)
                if encoded != stored:
                    temporary = self.path.with_suffix(".pending")
                    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
                    with os.fdopen(descriptor, "w", encoding="utf-8") as output:
                        output.write(encoded)
                        output.flush()
                        os.fsync(output.fileno())
                    temporary.replace(self.path)
            finally:
                if os.name == "nt":
                    lock.seek(0)
                    msvcrt.locking(lock.fileno(), msvcrt.LK_UNLCK, 1)
                else:
                    fcntl.flock(lock.fileno(), fcntl.LOCK_UN)

    def status(self):
        with self.transaction() as value:
            return {"connected": bool(value.get("access_token")), "refreshable": bool(value.get("refresh_token")), "expires_at": value.get("expires_at")}


class OAuth:
    def __init__(self, authorize_url, token_url, client_id, scopes=()):
        self.authorize_url = authorize_url
        self.token_url = token_url
        self.client_id = client_id
        self.scopes = scopes

    def authorize(self, context):
        with context.credentials.transaction() as value:
            value["state"] = secrets.token_urlsafe(24)
            parameters = {"response_type": "code", "client_id": self.client_id, "redirect_uri": context.config["redirect_uri"], "state": value["state"], "scope": " ".join(self.scopes)}
            return {"authorize_url": self.authorize_url + "?" + urllib.parse.urlencode(parameters), "state": value["state"]}

    def token_request(self, context, parameters):
        parameters = {"client_id": self.client_id, **parameters}
        if context.config.get("client_secret"):
            parameters["client_secret"] = context.config["client_secret"]
        request = urllib.request.Request(self.token_url, data=urllib.parse.urlencode(parameters).encode(), headers={"Accept": "application/json"})
        with urllib.request.urlopen(request, timeout=context.config.get("timeout", 60)) as response:
            return json.load(response)

    def save_tokens(self, value, tokens):
        if not tokens.get("access_token"):
            raise ValueError("OAuth response has no access_token")
        value["access_token"] = tokens["access_token"]
        if tokens.get("refresh_token"):
            value["refresh_token"] = tokens["refresh_token"]
        value["expires_at"] = time.time() + float(tokens["expires_in"]) if tokens.get("expires_in") is not None else 0
        value.pop("state", None)

    def exchange(self, context, code, state):
        with context.credentials.transaction() as value:
            if not state or state != value.get("state"):
                raise ValueError("OAuth state does not match this channel")
            tokens = self.token_request(context, {"grant_type": "authorization_code", "code": code, "redirect_uri": context.config["redirect_uri"]})
            self.save_tokens(value, tokens)
        return context.credentials.status()

    def access_token(self, context, force=False):
        with context.credentials.transaction() as value:
            if force or not value.get("access_token") or value.get("expires_at", 0) <= time.time() + 60:
                if not value.get("refresh_token"):
                    raise ValueError("OAuth authorization or refresh_token is required")
                tokens = self.token_request(context, {"grant_type": "refresh_token", "refresh_token": value["refresh_token"]})
                self.save_tokens(value, tokens)
            return value["access_token"]


@dataclasses.dataclass
class Response:
    status: int
    headers: dict
    body: object

    @classmethod
    def json(cls, value, status=200):
        return cls(status, {"Content-Type": "application/json"}, json.dumps(value, ensure_ascii=False).encode("utf-8"))

    @classmethod
    def resource(cls, resource, status=200, *, attachment=False):
        headers = {"Content-Type": resource.content_type}
        if attachment:
            headers["Content-Disposition"] = "attachment; filename*=UTF-8''" + urllib.parse.quote(resource.filename, safe="")
        return cls(status, headers, resource.open())


def sse_events(lines):
    event, data = "message", []
    for raw in lines:
        line = raw.decode("utf-8") if isinstance(raw, bytes) else raw
        line = line.rstrip("\r\n")
        if not line:
            if data:
                text = "\n".join(data)
                yield event, text if text == "[DONE]" else json.loads(text)
            event, data = "message", []
        elif line.startswith("event:"):
            event = line[6:].lstrip(" ")
        elif line.startswith("data:"):
            data.append(line[5:].lstrip(" "))


def standard_events(event, payload):
    if payload == "[DONE]":
        yield {"type": "done"}
        return
    kind = payload.get("type", event)
    if "error" in payload or kind in {"error", "response.failed", "response.incomplete"} or (kind == "response.completed" and payload.get("response", {}).get("status") in {"failed", "incomplete", "cancelled"}):
        yield {"type": "error", "data": payload}
        return
    if "choices" in payload:
        for choice in payload["choices"]:
            delta = choice.get("delta", {})
            for field, target in [("content", "text"), ("reasoning_content", "reasoning")]:
                if delta.get(field):
                    yield {"type": target, "text": delta[field], "index": choice.get("index", 0)}
            for tool in delta.get("tool_calls", []):
                yield {"type": "tool_delta", "data": tool}
        if payload.get("usage") is not None:
            yield {"type": "usage", "usage": payload["usage"]}
    elif kind in {"response.output_text.delta", "response.reasoning_summary_text.delta", "response.reasoning_text.delta"}:
        yield {"type": "text" if kind == "response.output_text.delta" else "reasoning", "text": payload.get("delta", "")}
    elif kind == "response.output_item.added" and payload.get("item", {}).get("type") == "function_call":
        yield {"type": "tool_start", "data": payload["item"]}
    elif kind == "response.function_call_arguments.delta":
        yield {"type": "tool_delta", "data": payload}
    elif kind == "response.completed":
        if payload.get("response", {}).get("usage") is not None:
            yield {"type": "usage", "usage": payload["response"]["usage"]}
        yield {"type": "done"}
    elif kind == "content_block_start" and payload.get("content_block", {}).get("type") == "tool_use":
        yield {"type": "tool_start", "data": payload["content_block"], "index": payload.get("index")}
    elif kind == "content_block_delta":
        delta = payload.get("delta", {})
        target = {"text_delta": "text", "thinking_delta": "reasoning", "input_json_delta": "tool_delta", "signature_delta": "signature"}.get(delta.get("type"))
        if target:
            yield {"type": target, "data": delta, "index": payload.get("index")}
    elif kind in {"message_start", "message_delta"}:
        usage = payload.get("message", payload).get("usage")
        if usage is not None:
            yield {"type": "usage", "usage": usage}
    elif kind == "message_stop":
        yield {"type": "done"}


class Channel:
    protocol = "openai_chat"
    oauth = None

    def __init__(self, manifest):
        self.manifest = manifest

    def headers(self, context):
        headers = {"Content-Type": "application/json"}
        if self.oauth is not None:
            headers["Authorization"] = f"Bearer {self.oauth.access_token(context)}"
        auth = context.authentication
        token = context.config.get(auth.get("config_key", "api_key"), "")
        if auth.get("type") == "bearer" and self.oauth is None:
            headers["Authorization"] = f"Bearer {token}"
        elif auth.get("type") == "api_key":
            headers[auth.get("header") or "x-api-key"] = token
        if self.protocol == "claude":
            headers["anthropic-version"] = "2023-06-01"
        return headers

    def prepare(self, context):
        return context.parameters

    def url(self, context, endpoint):
        base = context.config["base_url"].rstrip("/")
        if base.endswith(endpoint):
            return base
        return base + (endpoint[3:] if base.endswith("/v1") and endpoint.startswith("/v1/") else endpoint)

    def open_http(self, context, endpoint, body=None, *, form=None, multipart=None, binary=None, method=None, headers=None, timeout=None):
        if context.cancelled.is_set():
            raise InterruptedError("Request cancelled")
        if sum(value is not None for value in (body, form, multipart, binary)) > 1:
            raise ValueError("Select one HTTP request body format")
        outgoing = self.headers(context)
        if multipart is not None:
            data = Multipart(multipart)
            outgoing["Content-Type"] = data.content_type
            outgoing["Content-Length"] = str(data.size)
        elif form is not None:
            fields = []
            for name, value in form.items() if isinstance(form, dict) else form:
                for item in value if isinstance(value, list) else [value]:
                    fields.append((name, item if isinstance(item, str) else json.dumps(item, ensure_ascii=False)))
            data = urllib.parse.urlencode(fields).encode("utf-8")
            outgoing["Content-Type"] = "application/x-www-form-urlencoded"
        elif binary is not None:
            if isinstance(binary, Resource):
                data = binary.chunks()
                outgoing["Content-Type"] = binary.content_type
                outgoing["Content-Length"] = str(binary.size)
            elif isinstance(binary, (bytes, bytearray)):
                data = binary
                outgoing["Content-Type"] = "application/octet-stream"
            else:
                raise TypeError("Binary body must be bytes or a request Resource")
        else:
            data = None if body is None else json.dumps(body, ensure_ascii=False).encode("utf-8")
        outgoing.update(headers or {})
        request = urllib.request.Request(self.url(context, endpoint), data=data,
            headers=outgoing, method=method or ("GET" if data is None else "POST"))
        try:
            return urllib.request.urlopen(request, timeout=timeout if timeout is not None else context.config.get("timeout", 60))
        except urllib.error.HTTPError as response:
            return response

    def invoke(self, context):
        format_name = context.request.get("format", "json")
        if format_name == "binary":
            options = {"binary": context.body_resource}
        elif format_name in ("form", "multipart"):
            options = {format_name: self.prepare(context)}
        else:
            options = {"body": self.prepare(context)}
        response = self.open_http(context, context.request["endpoint"], method=context.request.get("method", "POST"), **options)
        headers = {name: response.headers[name] for name in ("Content-Type", "Content-Disposition", "Retry-After", "Location") if name in response.headers}
        return Response(response.status, headers, response)

    def read_json(self, response):
        with response:
            content = response.read(REQUEST_LIMIT + 1)
            if len(content) > REQUEST_LIMIT:
                raise ValueError("Upstream JSON exceeds SDK body limit")
            if response.status >= 400:
                raise RuntimeError(f"Upstream HTTP {response.status}")
            return json.loads(content)

    def download(self, context, endpoint, *, filename="resource"):
        with self.open_http(context, endpoint) as response:
            if response.status >= 400:
                raise RuntimeError(f"Resource download HTTP {response.status}")
            return context.resources.add(response, filename=filename, content_type=response.headers.get("Content-Type", "application/octet-stream"))

    def poll_task(self, context, endpoint, *, status=lambda value: value["status"], completed=("completed", "succeeded"), failed=("failed", "cancelled"), interval=1, timeout=120, on_update=None):
        if interval <= 0 or timeout <= 0:
            raise ValueError("Polling interval and timeout must be positive")
        deadline = time.monotonic() + timeout
        while True:
            if context.cancelled.is_set():
                raise InterruptedError("Task polling cancelled")
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("Task polling timed out")
            result = self.read_json(self.open_http(context, endpoint, timeout=min(remaining, context.config.get("timeout", 60))))
            state = status(result)
            if on_update is not None:
                on_update(result)
            if state in completed:
                return result
            if state in failed:
                raise RuntimeError(f"Upstream task {state}")
            if context.cancelled.wait(min(interval, max(0, deadline - time.monotonic()))):
                raise InterruptedError("Task polling cancelled")

    def models(self, context):
        if not self.manifest["capabilities"].get("model_discovery"):
            return self.manifest.get("models", [])
        with self.open_http(context, "/v1/models") as response:
            if response.status >= 400:
                raise RuntimeError(f"model discovery HTTP {response.status}")
            return [{"id": row["id"], "name": row.get("name", row["id"])} for row in json.load(response)["data"]]

    def oauth_operation(self, context, operation):
        if self.oauth is None:
            raise ValueError("This channel has no OAuth extension")
        if operation["action"] == "authorize":
            return self.oauth.authorize(context)
        if operation["action"] == "exchange":
            return self.oauth.exchange(context, operation["code"], operation["state"])
        if operation["action"] == "refresh":
            self.oauth.access_token(context, force=True)
            return context.credentials.status()
        if operation["action"] == "status":
            return context.credentials.status()
        raise ValueError("Unknown OAuth action")


class OpenAIChat(Channel):
    protocol = "openai_chat"


class Responses(Channel):
    protocol = "responses"


class Claude(Channel):
    protocol = "claude"


class NativeProxy(Channel):
    protocol = "native"

    class NoRedirect(urllib.request.HTTPRedirectHandler):
        def redirect_request(self, request, response, code, message, headers, target):
            return None

    @cached_property
    def transport(self):
        return urllib.request.build_opener(self.NoRedirect())

    def url(self, context, endpoint):
        return context.config["base_url"].rstrip("/") + endpoint

    def invoke(self, context):
        if context.request.get("format") != "native":
            raise RequestParameterError("NativeProxy requires a native gateway route")
        if context.cancelled.is_set():
            raise InterruptedError("Request cancelled")
        headers = {name.lower(): value for name, value in context.request.get("headers", [])}
        authentication = {name.lower(): value for name, value in self.headers(context).items() if name.lower() != "content-type"}
        headers.update(authentication)
        query = context.request.get("query", "")
        if context.authentication.get("type") == "query":
            credential = context.config[context.authentication["config_key"]]
            parameter = urllib.parse.urlencode({context.authentication["query"]: credential})
            query = query + "&" + parameter if query else parameter
        target = self.url(context, context.request["endpoint"])
        if query:
            target += "?" + query
        resource = context.body_resource
        if resource.size:
            headers.setdefault("content-type", resource.content_type)
        headers["content-length"] = str(resource.size)
        request = urllib.request.Request(target, data=resource.chunks() if resource.size else None,
            headers=headers, method=context.request["method"])
        try:
            response = self.transport.open(request, timeout=context.config.get("timeout", 60))
        except urllib.error.HTTPError as error:
            response = error
        return Response(response.status, list(response.headers.items()), response)


def _watch_parent(parent):
    if os.name == "nt":
        import ctypes
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.OpenProcess.restype = ctypes.c_void_p
        kernel.WaitForSingleObject.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
        kernel.CloseHandle.argtypes = [ctypes.c_void_p]
        handle = kernel.OpenProcess(0x00100000, False, parent)
        if not handle:
            os._exit(0)
        kernel.WaitForSingleObject(handle, 0xFFFFFFFF)
        kernel.CloseHandle(handle)
        os._exit(0)
    else:
        while True:
            try:
                os.kill(parent, 0)
            except ProcessLookupError:
                os._exit(0)
            time.sleep(1)


def read_envelope(source, size, headers, resources):
    if headers.get("Content-Type", "").split(";", 1)[0].strip().lower() != "application/vnd.miora.resource+octet-stream":
        return json.loads(b"".join(read_exact(source, size)))
    envelope_size = int(headers["x-miora-envelope-length"])
    if not 0 < envelope_size <= min(size, 1024 * 1024):
        raise RequestParameterError("Invalid plugin resource envelope size")
    envelope = json.loads(b"".join(read_exact(source, envelope_size)))
    metadata = envelope["request"]["body_resource"]
    if metadata["size"] != size - envelope_size:
        raise RequestParameterError("Plugin resource length does not match envelope")
    resources.add(read_exact(source, metadata["size"]), resource_id=metadata["resource_id"], content_type=metadata["content_type"])
    return envelope


def serve(channel):
    token = os.environ["MIORA_PLUGIN_SERVICE_TOKEN"]

    class Handler(http.server.BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *args):
            pass

        def authenticated(self):
            if self.headers.get("Authorization") != f"Bearer {token}":
                self.json_response(401, {"error": {"message": "Plugin service authorization required"}})
                return False
            return True

        def json_response(self, status, body):
            encoded = json.dumps(body, ensure_ascii=False).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(encoded)))
            self.send_header("Connection", "close")
            self.end_headers()
            self.wfile.write(encoded)
            self.close_connection = True

        def do_GET(self):
            if not self.authenticated():
                return
            if self.path == "/health":
                self.json_response(200, {"ok": True, "plugin_id": channel.manifest["id"], "version": channel.manifest["version"], "api_version": 1})
            else:
                self.json_response(404, {"error": {"message": "Unknown plugin endpoint"}})

        def do_POST(self):
            if not self.authenticated():
                return
            response = None
            context = None
            resources = ResourceStore()
            started = False
            try:
                size = int(self.headers.get("Content-Length", "0"))
                if not 0 < size <= REQUEST_LIMIT:
                    self.json_response(413, {"error": {"message": "Plugin request exceeds body limit"}})
                    return
                envelope = read_envelope(self.rfile, size, self.headers, resources)
                context = Context(envelope.get("request", {}), envelope["channel"]["config"], envelope["channel"].get("name", ""), envelope.get("authentication", {}), resources=resources, request_schema=channel.manifest.get("request_schema", []))
                if self.path == "/models":
                    self.json_response(200, {"models": channel.models(context)})
                    return
                if self.path == "/oauth":
                    self.json_response(200, channel.oauth_operation(context, envelope["oauth"]))
                    return
                if self.path != "/execute":
                    self.json_response(404, {"error": {"message": "Unknown plugin endpoint"}})
                    return
                if context.request.get("format") != "native":
                    context.parameters
                response = channel.invoke(context)
                self.send_response(response.status)
                response_headers = list(response.headers.items()) if isinstance(response.headers, dict) else response.headers
                connection_headers = {part.strip().lower() for name, value in response_headers if name.lower() == "connection" for part in value.split(",")}
                for name, value in response_headers:
                    if name.lower() not in {"connection", "keep-alive", "proxy-authenticate", "proxy-authorization", "te", "trailer", "transfer-encoding", "upgrade", "content-length"} | connection_headers:
                        self.send_header(name, value)
                self.send_header("Connection", "close")
                self.end_headers()
                started = True
                self.close_connection = True
                if isinstance(response.body, (bytes, bytearray)):
                    self.wfile.write(response.body)
                elif hasattr(response.body, "read1"):
                    while not context.cancelled.is_set():
                        chunk = response.body.read1(16384)
                        if not chunk:
                            break
                        self.wfile.write(chunk)
                        self.wfile.flush()
                elif hasattr(response.body, "read"):
                    while not context.cancelled.is_set():
                        chunk = response.body.read(16384)
                        if not chunk:
                            break
                        self.wfile.write(chunk)
                        self.wfile.flush()
                else:
                    for chunk in response.body:
                        if context.cancelled.is_set():
                            break
                        self.wfile.write(chunk)
                        self.wfile.flush()
            except (BrokenPipeError, ConnectionResetError):
                if context is not None:
                    context.cancelled.set()
            except RequestParameterError as error:
                if not started:
                    self.json_response(400, {"error": {"message": str(error), "type": "plugin_request_parameter"}})
            except Exception as error:
                print(f"Plugin request failed: {type(error).__name__}", flush=True)
                if not started:
                    self.json_response(502, {"error": {"message": f"Plugin request failed: {type(error).__name__}"}})
                self.close_connection = True
            finally:
                if response is not None and hasattr(response.body, "close"):
                    response.body.close()
                resources.close()

    if os.environ.get("MIORA_PLUGIN_PARENT_PID"):
        threading.Thread(target=_watch_parent, args=(int(os.environ["MIORA_PLUGIN_PARENT_PID"]),), daemon=True).start()
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    server.daemon_threads = True
    ready = pathlib.Path(os.environ["MIORA_PLUGIN_READY_FILE"])
    temporary = ready.with_suffix(".pending")
    temporary.write_text(json.dumps({"port": server.server_port}), encoding="utf-8")
    temporary.replace(ready)
    server.serve_forever()
