import base64
import copy
import dataclasses
import email.parser
import email.policy
import json
import pathlib
import secrets
import tempfile
import urllib.parse


RESOURCE_LIMIT = 128 * 1024 * 1024
REQUEST_LIMIT = 64 * 1024 * 1024
FIELD_LIMIT = 1024 * 1024


class RequestParameterError(ValueError):
    pass


@dataclasses.dataclass(frozen=True)
class Resource:
    resource_id: str
    filename: str
    content_type: str
    size: int
    _store: object = dataclasses.field(repr=False, compare=False)

    def metadata(self):
        return {key: getattr(self, key) for key in ("resource_id", "filename", "content_type", "size")}

    def open(self):
        return self._store.open(self)

    def chunks(self):
        with self.open() as source:
            while chunk := source.read(16384):
                yield chunk


class ResourceStore:
    def __init__(self, limit=RESOURCE_LIMIT):
        self._directory = None
        self._resources = {}
        self._used = 0
        self.limit = limit

    def close(self):
        if self._directory is not None:
            self._directory.cleanup()
        self._resources.clear()

    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.close()

    def add(self, source, *, filename="resource", content_type="application/octet-stream", resource_id=None):
        if len(self._resources) >= 128:
            raise RequestParameterError("Too many request resources")
        resource_id = resource_id or secrets.token_urlsafe(18)
        if resource_id in self._resources or not resource_id.replace("-", "").replace("_", "").isalnum():
            raise RequestParameterError("Invalid or duplicate resource ID")
        if self._directory is None:
            self._directory = tempfile.TemporaryDirectory(prefix="miora-resources-")
        path = pathlib.Path(self._directory.name) / resource_id
        if isinstance(source, (bytes, bytearray)):
            source = (source,)
        elif hasattr(source, "read"):
            reader = source
            source = iter(lambda: reader.read(16384), b"")
        size = 0
        try:
            with path.open("xb") as output:
                for chunk in source:
                    if self._used + size + len(chunk) > self.limit:
                        raise RequestParameterError("Request resource capacity exceeded")
                    output.write(chunk)
                    size += len(chunk)
        except BaseException:
            path.unlink(missing_ok=True)
            raise
        filename = filename.replace("\\", "/").rsplit("/", 1)[-1]
        resource = Resource(resource_id, filename, content_type, size, self)
        self._resources[resource_id] = resource
        self._used += size
        return resource

    def get(self, reference):
        resource_id = reference.resource_id if isinstance(reference, Resource) else reference.get("resource_id") if isinstance(reference, dict) else reference
        if not isinstance(resource_id, str):
            raise RequestParameterError("Invalid request resource reference")
        resource = self._resources.get(resource_id)
        if resource is None or isinstance(reference, Resource) and reference._store is not self:
            raise RequestParameterError("Resource is not owned by this request")
        return resource

    def open(self, reference):
        resource = self.get(reference)
        return (pathlib.Path(self._directory.name) / resource.resource_id).open("rb")

    def from_data_url(self, value, filename="image"):
        prefix, encoded = value.split(",", 1)
        if not prefix.startswith("data:") or not prefix.endswith(";base64"):
            raise RequestParameterError("Expected a base64 data URL")
        if len(encoded) > (self.limit - self._used) * 4 // 3 + 4:
            raise RequestParameterError("Request resource capacity exceeded")
        return self.add(base64.b64decode(encoded, validate=True), filename=filename, content_type=prefix[5:-7])


def read_exact(source, size):
    remaining = size
    while remaining:
        chunk = source.read(min(16384, remaining))
        if not chunk:
            raise RequestParameterError("Truncated request resource")
        remaining -= len(chunk)
        yield chunk


class MultipartReader:
    def __init__(self, source, content_type):
        message = email.parser.BytesHeaderParser().parsebytes(b"Content-Type: " + content_type.encode("ascii") + b"\r\n\r\n")
        boundary = message.get_boundary()
        if not boundary or len(boundary) > 70:
            raise RequestParameterError("Multipart boundary is missing or too long")
        self.source = source
        self.boundary = boundary.encode("ascii")
        self.buffer = b""
        self.finished = False
        first = self.line()
        if first == b"--" + self.boundary + b"--\r\n":
            self.finished = True
        elif first != b"--" + self.boundary + b"\r\n":
            raise RequestParameterError("Invalid multipart opening boundary")

    def line(self):
        while b"\n" not in self.buffer:
            chunk = self.source.read(16384)
            if not chunk or len(self.buffer) > 16384:
                raise RequestParameterError("Invalid multipart header")
            self.buffer += chunk
        line, self.buffer = self.buffer.split(b"\n", 1)
        if len(line) > 16384:
            raise RequestParameterError("Multipart header is too long")
        return line + b"\n"

    def chunks(self):
        marker = b"\r\n--" + self.boundary
        keep = len(marker) + 2
        while True:
            position = self.buffer.find(marker)
            while position >= 0 and len(self.buffer) >= position + keep:
                suffix = self.buffer[position + len(marker):position + keep]
                if suffix in (b"\r\n", b"--"):
                    yield self.buffer[:position]
                    self.buffer = self.buffer[position + keep:]
                    self.finished = suffix == b"--"
                    return
                position = self.buffer.find(marker, position + 1)
            if len(self.buffer) > keep:
                yield self.buffer[:-keep]
                self.buffer = self.buffer[-keep:]
            chunk = self.source.read(16384)
            if not chunk:
                raise RequestParameterError("Multipart closing boundary is missing")
            self.buffer += chunk

    def fields(self, resources):
        fields = []
        while not self.finished:
            if len(fields) >= 128:
                raise RequestParameterError("Too many multipart fields")
            lines = []
            size = 0
            while (line := self.line()) != b"\r\n":
                size += len(line)
                if size > 16384:
                    raise RequestParameterError("Multipart headers are too large")
                lines.append(line)
            headers = email.parser.BytesHeaderParser(policy=email.policy.default).parsebytes(b"".join(lines) + b"\r\n")
            name = headers.get_param("name", header="content-disposition")
            if not name:
                raise RequestParameterError("Multipart field has no name")
            filename = headers.get_filename()
            if filename is not None:
                value = resources.add(self.chunks(), filename=filename, content_type=headers.get_content_type())
            else:
                content = bytearray()
                for chunk in self.chunks():
                    if len(content) + len(chunk) > FIELD_LIMIT:
                        raise RequestParameterError("Multipart text field is too large")
                    content.extend(chunk)
                value = content.decode(headers.get_content_charset() or "utf-8")
            fields.append((name, value))
        return fields


def form_fields(resource, resources):
    with resource.open() as source:
        if resource.content_type.split(";", 1)[0].strip().lower() == "application/x-www-form-urlencoded":
            if resource.size > FIELD_LIMIT:
                raise RequestParameterError("Form fields are too large")
            return urllib.parse.parse_qsl(source.read().decode("utf-8"), keep_blank_values=True, max_num_fields=128)
        return MultipartReader(source, resource.content_type).fields(resources)


def request_parameters(request, schema, resources):
    format_name = request.get("format", "json")
    if format_name in ("form", "multipart"):
        fields = form_fields(resources.get(request["body_resource"]), resources)
        values = {}
        for name, value in fields:
            if name in values:
                values[name] = values[name] + [value] if isinstance(values[name], list) else [values[name], value]
            else:
                values[name] = value
        values["model"] = request["model"]
    else:
        values = dict(request.get("body") or {})
    query_parameters = request.get("query_parameters", {})
    query_keys = query_parameters.keys() - values.keys()
    values = {**query_parameters, **values}
    for field in schema:
        key, kind = field["key"], field["type"]
        if key not in values:
            if "default" in field:
                values[key] = copy.deepcopy(field["default"])
            elif field.get("required"):
                raise RequestParameterError(f"Missing plugin request parameter: {key}")
            else:
                continue
        value = values[key]
        if (format_name in ("form", "multipart") or key in query_keys) and isinstance(value, str) and kind in ("integer", "number", "boolean", "object", "array"):
            try:
                value = json.loads(value)
            except ValueError:
                raise RequestParameterError(f"Invalid plugin request parameter: {key}") from None
        matches = {"string": lambda: isinstance(value, str), "integer": lambda: type(value) is int, "number": lambda: type(value) in (int, float), "boolean": lambda: type(value) is bool, "object": lambda: isinstance(value, dict), "array": lambda: isinstance(value, list), "resource": lambda: isinstance(value, (Resource, dict))}
        if kind not in matches or not matches[kind]() or field.get("enum") and value not in field["enum"]:
            raise RequestParameterError(f"Invalid plugin request parameter: {key}")
        values[key] = resources.get(value) if kind == "resource" else value
    return values


def _quoted(value):
    if any(char in value for char in ("\r", "\n", "\0")):
        raise RequestParameterError("Invalid multipart field metadata")
    return value.replace("\\", "\\\\").replace('"', '\\"')


class Multipart:
    def __init__(self, fields):
        self.boundary = "miora-" + secrets.token_hex(18)
        self.content_type = "multipart/form-data; boundary=" + self.boundary
        self.parts = []
        for name, value in fields.items() if isinstance(fields, dict) else fields:
            for item in value if isinstance(value, list) else [value]:
                header = f'--{self.boundary}\r\nContent-Disposition: form-data; name="{_quoted(name)}"'
                if isinstance(item, Resource):
                    header += f'; filename="{_quoted(item.filename)}"\r\nContent-Type: {_quoted(item.content_type)}'
                    body = item
                else:
                    body = (item if isinstance(item, str) else json.dumps(item, ensure_ascii=False)).encode("utf-8")
                self.parts.append(((header + "\r\n\r\n").encode("utf-8"), body))
        self.ending = f"--{self.boundary}--\r\n".encode()
        self.size = sum(len(header) + (body.size if isinstance(body, Resource) else len(body)) + 2 for header, body in self.parts) + len(self.ending)

    def __iter__(self):
        for header, body in self.parts:
            yield header
            if isinstance(body, Resource):
                yield from body.chunks()
            else:
                yield body
            yield b"\r\n"
        yield self.ending
