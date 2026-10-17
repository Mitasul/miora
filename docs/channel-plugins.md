# Channel plugins (protocol / SDK v1)

Miora supports built-in Rust adapters, uploaded managed Python ZIP packages, and
independent HTTP services. For upload-and-run installation, SDK extension points,
OAuth, configuration aliases and process/version management, see the
[Python SDK guide](../sdk/python/README.md). This page documents the retained
independent-service contract; it does not require managed plugins to run a
manually deployed service.

Miora can execute an independently deployed channel service without adding a
supplier-specific `engine` branch to the gateway. The service receives an
explicit, versioned JSON envelope and returns the selected downstream protocol
(JSON, SSE or binary). Non-JSON request transport is an explicit opt-in extension
described below. Scheduling, API-key authorization, retries, hedging, timeouts,
usage observation and request logs remain in the Rust gateway.

This is not an interceptor masquerading as a provider. A **channel plugin** owns
model discovery, upstream authentication and execution. **Header interceptors**
are a separate, ordered extension registry with narrowly bounded permissions.

## Multimodal opt-in

`capabilities.request_formats` defaults to `["json"]`. Plugins may additionally
declare `form`, `multipart` and `binary`; the image-edit and audio-transcription
endpoints are supported without replacing their existing Rust implementations.
`request_schema` uses the configuration-field shape to declare provider-owned
parameters (including `resource` fields). Those parameters bypass generic JSON
policy mutation; the plugin SDK validates and converts them. Only declared query
parameters are included as `request.query_parameters`; body fields take precedence.

## Native authentication proxy opt-in

Declare `manifest.native_proxy` to mount provider-native HTTP routes under
`/native/<percent-encoded-channel-name>/...`. The full declaration, authorization
model sources and the managed `NativeProxy` SDK are documented in the
[Python SDK guide](../sdk/python/README.md#供应商原生协议认证代理).
This mode is also available to independently deployed services; it does not
require conversion to an OpenAI request or response.

The existing resource RPC carries `request.format: "native"`, the original
`request.endpoint`, raw credential-stripped `request.query`, business
`request.headers` as `[name, value]` pairs, and `request.body_resource` followed
by the unchanged body bytes. `request.model` is an authorization sidecar, not
an instruction to rewrite that resource. The channel configuration contains
the scheduled upstream credential, never the caller credential. Services must
inject that credential according to `authentication` and must not follow
redirects with it. HTTP status, business response headers and binary/SSE bodies
are relayed without generic protocol transformation or automatic retries.
The gateway continues to apply existing key/model/group authorization, rate
admission, channel credential selection, resource limits, logging and pinned
plugin execution versions. It does not infer arbitrary provider business
completion from HTTP 200 or supply WebSocket upgrade support.

For non-JSON execution, the internal service request has Content-Type
`application/vnd.miora.resource+octet-stream`, a Content-Length, and
`x-miora-envelope-length`. Its body is exactly the UTF-8 JSON envelope followed
by the original resource bytes. `request.format` identifies the client encoding;
`request.body_resource` contains `resource_id`, `content_type`, and byte `size`.
There is no filesystem path, URL granting host access, base64 copy or checksum.
The existing service bearer authentication and version/revision envelope remain.
Legacy JSON-only services retain the original JSON transport and envelope shape.

Independent services opting into these formats must implement this transport or
reuse `read_envelope` and the resource helpers in the Python SDK. The SDK exposes
request-scoped files, streaming multipart encoding, binary/JSON responses and
cooperative asynchronous-task polling. It does not provide persistent public
file URLs or a durable job queue. See the [SDK guide](../sdk/python/README.md)
for limits, lifecycle and examples.

## File installation in the admin UI

For an independent service, the plugin manager accepts a native `.miora.json` file. Upload, review its
name/ID/version, and install; updates use the same upload flow. There is no
installation configuration form. Supplier URLs, API keys and plugin parameters
belong to the channel editor. Built-in adapters are listed separately.

The `.miora.json` file is a service descriptor; a managed executable package is uploaded separately as ZIP:

```json
{
  "format": "miora.plugin.v1",
  "manifest": {
    "id": "example",
    "name": "Example",
    "version": "1.0.0",
    "api_version": 1,
    "capabilities": {"endpoints": ["/v1/chat/completions"]}
  },
  "endpoint": "https://plugin.example.com",
  "service_auth": {"type": "bearer", "token": "ISSUED-SERVICE-CREDENTIAL"}
}
```

The publisher supplies an already-running service and its independent service
credential in the file. Treat that file as sensitive; do not commit credentials.
Optional `config` contains publisher defaults, never a required installation
step. Manifest `config_schema` fields are configured per channel, including
required fields. This format and the service contract are Miora-native, not a
compatibility implementation of another project's protocol.

`GET /admin/api/plugins/template` downloads an example. Authenticated
`POST /admin/api/plugins/upload` accepts multipart `file` and optional `options`
JSON containing `expected_revision` (0 for new installs), `trusted_code` (managed
packages only), and `allow_version_change` (explicit replacement/downgrade confirmation). Cookies and CSRF
are required. Descriptors are limited to 256 KiB, ZIPs to 16 MiB and complete multipart bodies to 17 MiB;
duplicate/unknown fields and unsupported formats fail before publication.
Existing trust, health-handshake and revision-CAS checks remain authoritative.
The existing JSON registration API is retained for automation. Uploading does not
execute code or provision an independent service. Managed ZIP installation does
run trusted code and dependencies, following its explicit confirmation; it does
not inherit permission to use arbitrary independent-service origins.

## Quick start: entirely local, no paid upstream

The files in `examples/plugins/echo/` use only Python 3.10+ standard library.

1. Start the independent service:

   ```sh
   python examples/plugins/echo/server.py --port 8765 --token local-demo-service-token
   ```

2. Before starting Miora set:

   ```text
   MIORA_PLUGIN_ALLOW_LOOPBACK_HTTP=true
   MIORA_PLUGIN_STORE_DIR=data/channel-plugins
   MIORA_ADMIN_USERNAME=admin
   MIORA_ADMIN_PASSWORD=<your own strong password>
   MIORA_ADMIN_SECURE_COOKIE=false
   MIORA_ADMIN_ORIGIN=http://127.0.0.1:8000
   ```

   `MIORA_ADMIN_SECURE_COOKIE=false` is for this loopback HTTP demo only; use
   HTTPS and secure cookies outside local development.

   The loopback exception is opt-in and permits **literal loopback IPs**, not
   `localhost` or arbitrary hostnames. `http://127.0.0.1:8765` is suitable here.
   Administrator setup, cookies and CSRF are documented by the admin module.

3. Register the plugin in the Plugins admin page, or set `MIORA_ADMIN_PASSWORD`
   in the shell and run:

   ```sh
   python examples/plugins/echo/register.py --gateway http://127.0.0.1:8000
   ```

   Set `ECHO_SERVICE_TOKEN` if the service uses a different token. The helper
   logs in, uses the HttpOnly cookie and `X-CSRF-Token`, performs revision-CAS
   registration and logs out. It does not create an inference API key.

4. Add a channel in the admin UI or in your configuration source:

   ```yaml
   providers:
     - provider: local-echo
       channel_plugin: echo
       plugin_config:
         prefix: Demo
       model:
         - echo-model
   api_keys:
     - api: YOUR-OWN-INFERENCE-KEY
       role: user
       model:
         - local-echo/*
   ```

   A service channel needs neither `base_url` nor `engine`; the endpoint belongs
   to the trusted registration. Models must be explicitly saved. Model discovery
   is a deliberate admin action, not an implicit outbound request on config load.
   Existing `engine: plugin:echo` is also accepted. The compiler preserves
   `channel_plugin` and `plugin_config` in provider preferences and uses a
   non-secret scheduler placeholder when the service channel needs no API key.

5. Call normal `/v1/chat/completions`, `/v1/responses`, or `/v1/embeddings` with
   `model: echo-model`. Chat and Responses both support `stream: true` in this
   demo. SSE is sent progressively and the demo makes no upstream network call.

To exercise an independent-service update, keep the old service running and
start a second version-specific endpoint, then register it:

```sh
python examples/plugins/echo/server.py --port 8766 --token local-demo-service-token
python examples/plugins/echo/register.py --endpoint http://127.0.0.1:8766 --version 2.0.0 --prefix Updated
```

The demo is a configuration-versioned engine that accepts arbitrary manifest
versions. A real multi-version service must retain the implementations selected
by `plugin.version` and `plugin.revision`; do not deploy incompatible code behind
the same endpoint and silently stop supporting versions with active leases.
Updates must use a distinct immutable execution endpoint. Reusing a retained
endpoint for another registration is rejected; a descriptor alone cannot retain
an implementation that the remote deployment has already overwritten.

## Manifest

`examples/plugins/echo/manifest.json` is a complete, machine-readable example.
The registration contains:

```json
{
  "manifest": {
    "id": "echo",
    "name": "Echo",
    "version": "1.0.0",
    "api_version": 1,
    "description": "An independent channel service",
    "config_schema": [
      {"key":"prefix","label":"Reply prefix","type":"string","default":"Echo","required":false,"secret":false},
      {"key":"api_key","label":"Upstream credential","type":"string","required":true,"secret":true}
    ],
    "auth": {"type":"bearer","config_key":"api_key"},
    "capabilities": {
      "endpoints":["/v1/chat/completions","/v1/responses"],
      "streaming":true,
      "model_discovery":true
    },
    "models":[{"id":"example-model","name":"Example"}]
  },
  "endpoint":"https://plugins.example.net/echo-v1",
  "config":{},
  "service_auth":{"type":"bearer","token":"INDEPENDENT-SERVICE-CREDENTIAL"},
  "expected_revision":0
}
```

IDs and field keys use lowercase ASCII letters, digits, `_` and `-`, at most 64
characters. `builtin-*` IDs are reserved. The schema is an array, not JSON Schema:

- `type`: `string`, `integer`, `number`, `boolean`, `object` or `array`.
- `required`: enforced on the merged effective execution/discovery config.
  Registration may omit channel-specific required values.
- `default`: optional non-secret default, validated against type and enum.
- `enum`: optional array of allowed typed values.
- `secret`: hide values in management views. Secret defaults/enums are rejected.
  Unknown fields are also masked on read; a field classified secret in **any**
  retained version remains masked after updates and rollbacks. Secret-to-public
  schema updates are rejected.
- Unknown manifest fields, unknown config keys, invalid types, unsupported
  protocol versions and unsupported endpoint capabilities are rejected.

Configuration precedence: manifest defaults, registration `config`, channel
`plugin_config`, then the scheduled channel API key in the declared
`auth.config_key` (when a real key was configured). Gateway inference caller
credentials, admin cookies and CSRF tokens are never included.

`auth.type` is `none`, `bearer`, or `api_key`. Non-`none` references a secret
string schema field by `config_key`; `api_key` also declares its upstream header.
This is the service's upstream-authentication contract, **not** its control
channel credential. The service applies the declared method to its own upstream.

## Service endpoints

All paths are relative to the registered endpoint. HTTP redirects are disabled
and system proxy settings are not used by the plugin client.

### `GET /health`

Registration/update, enabling, and rollback check health before publishing a
candidate. Health receives only the independent service bearer and
`X-Miora-Plugin-Version: <candidate manifest version>`, not channel secrets.

```json
{"ok":true,"api_version":1,"plugin_id":"echo","version":"1.0.0"}
```

The ID, version and protocol must match. The service may select the requested
immutable implementation using the version header. A 10-second total deadline
and 16 KiB response limit apply. Failure, malformed output, identity mismatch,
redirect or unavailable service leaves the previous registry state intact.
Revision CAS is repeated after the network check, preventing lost updates.
Disabling does not require a healthy service. Startup restoration validates
stored data and endpoint trust but does not require all services to be online.

### `POST /models`

The envelope contains `api_version`, `plugin: {id,version,revision}`,
`channel: {config}`, and `authentication`. Return:

```json
{"models":[{"id":"echo-model","name":"Echo Model"}]}
```

Discovery uses a 15-second total deadline and a 1 MiB response limit. Up to 4096
models are accepted. If `model_discovery` is false, declared `manifest.models`
are returned without an outbound call. Discovered models are **not** silently
written to channel configuration; the administrator chooses what to save.

### `POST /execute`

```json
{
  "api_version":1,
  "plugin":{"id":"echo","version":"1.0.0","revision":1},
  "request":{
    "id":"request-id",
    "method":"POST",
    "endpoint":"/v1/chat/completions",
    "model":"upstream-model",
    "requested_model":"public-alias",
    "stream":true,
    "body":{"model":"upstream-model","messages":[{"role":"user","content":"hello"}],"stream":true}
  },
  "channel":{"name":"my-channel","config":{"prefix":"Echo"}},
  "authentication":{"type":"none","config_key":"","header":""}
}
```

The HTTP request is always POST to `/execute`, with the service bearer (when
configured), `Content-Type: application/json`, and
`X-Miora-Plugin-Revision: <execution revision>`. The original caller headers and
query string are not forwarded. The body model is the routed upstream model.

Return the **requested downstream protocol**, not a provider-specific response
or another wrapper. Examples:

- Chat nonstream: OpenAI-style `chat.completion`, choices/message and usage.
- Chat stream: `text/event-stream`, chat delta events and `data: [DONE]`.
- Responses: native Responses JSON/events including terminal
  `response.completed`. It is not reinterpreted as Chat.
- Embeddings/images/moderation/search: JSON matching the declared endpoint.
- HTTP failures: non-2xx status, which participates in normal routing retry
  policy. For safety, external error bodies are replaced with a status-only
  message before logging/returning; services must not echo secret envelopes.

Execution uses the resolved core connect/total/first-byte/idle timeout policy,
including explicitly disabled limits. Its dedicated no-redirect/no-proxy client
pool is keyed by the resolved connect timeout; there is no hidden global
stalled-read cap. Management health/discovery have their own bounded deadlines.
Core stream first-byte/idle guards still apply. The service is responsible for
wire-protocol correctness and accurate usage. No gateway-side background spawn
or process execution is involved.

## Registry, revisions and request leases

- Admin mutation CAS uses `plugin.revision`, initially 1. First registration uses
  `expected_revision: 0`. Every successful update, enable, disable or rollback
  increases this management revision; conflicts return HTTP 409.
- Each new registration/update creates an immutable execution version. Startup
  loads retained versions and their active/enabled state.
- A rollback moves `active` to an existing immutable execution revision and
  advances the management revision. It does **not** fabricate a new execution
  version and remains possible at the history limit.
- Inference captures an immutable plugin/interceptor snapshot at request entry,
  before asynchronous request processing. All retries and hedges use that same
  snapshot; they cannot drift to a newly registered version halfway through.
- The outgoing response `Body` holds an Arc lease through EOF, body error or
  cancellation/drop. Updating affects new requests. Disabling excludes new
  request snapshots but never aborts an existing request/stream.
- `versions[].leases` counts live immutable snapshot ownership groups, not exact
  HTTP requests. A pinned request conservatively retains all enabled candidates.
  `draining` means an inactive/disabled version is still referenced.
- Responses include `X-Miora-Plugin-Id`, `X-Miora-Plugin-Revision` and
  `X-Miora-Plugin-Version`. `channel_plugin_dispatch` structured events contain
  only ID/version/revision/channel/request ID, never config or service tokens.

## Management API

The parent admin handler authenticates a session and verifies CSRF before
calling `plugins::api::handle`. Inference API keys do not grant admin sessions.
All responses use `Cache-Control: no-store`.

| Method | Path | Body / result |
|---|---|---|
| GET | `/admin/api/plugins` | `{plugins:[...],trust:{allowed_origins,allow_loopback_http}}` |
| POST | `/admin/api/plugins` | Registration above; response `{plugin:...}` |
| POST | `/admin/api/plugins/{id}/enable` | `{expected_revision:N}` |
| POST | `/admin/api/plugins/{id}/disable` | `{expected_revision:N}` |
| POST | `/admin/api/plugins/{id}/rollback` | `{expected_revision:N,revision:historical_execution_revision}` |
| POST | `/admin/api/plugins/{id}/models` | Optional `{config:{...}}`; returns `{models:[...]}` |
| GET | `/admin/api/interceptors` | `{revision:N,interceptors:[...]}` |
| PUT | `/admin/api/interceptors` | `{expected_revision:N,interceptors:[...]}` |

Errors are `{error:{code,message}}`. Invalid manifests/config/health/trust return
400; revision conflicts 409; unknown resources 404; unsupported methods 405.
There is deliberately no delete endpoint: disable keeps audit/version history.
Secret values are `***`; an update or discovery override containing that marker
preserves the existing secret. A marker without a prior secret is invalid.

## Endpoint trust and persistence

Startup-only settings, never supplied by the registering manifest:

- `MIORA_PLUGIN_ALLOWED_ORIGINS`: comma-separated exact HTTPS origins, including
  nondefault port when applicable. Example: `https://plugins.example.net`.
- `MIORA_PLUGIN_ALLOW_LOOPBACK_HTTP`: false by default; explicit local-development
  exception for literal loopback HTTP endpoints.
- `MIORA_PLUGIN_STORE_DIR`: defaults to `data/channel-plugins`.

URLs with embedded credentials, query strings or fragments are rejected.
HTTPS services require an independent nonempty `service_auth.type: bearer` and
`token`; neither gateway caller bearer nor admin session is reused. Loopback
HTTP may explicitly use `none`, though the demo still requires bearer auth.
Trust is rechecked on restoring every retained version. Tightening the allowlist
so a retained endpoint becomes untrusted fails startup closed; migrate/archive
the protected store deliberately instead of silently bypassing policy.

The store uses immutable `revision-00000000000000000001.json` generations, each a
complete versioned snapshot. A create-new temporary file is flushed before
rename to a previously nonexistent generation; only then is in-memory state
published. A persistence error cannot swap the active version. This design works
with Windows rename semantics as well as Unix. Incomplete `.tmp` files are not
loaded. The highest committed generation is validated at startup; corrupted
latest data is not silently replaced with stale state.

**The store contains plaintext channel/service secrets.** Unix revision files
are created mode 0600; Windows deployments must set a restrictive directory ACL.
Keep the directory out of public web roots/source control.
Use one gateway writer per directory: this is not a distributed registry or a
cross-process transaction coordinator. Histories are bounded to 128 plugins,
100 execution versions per plugin, 64 interceptors, and a 64 MiB snapshot.
Reaching the version cap rejects new updates but never blocks disable/rollback.
After a new complete snapshot is published, prior committed generation files
are removed. Normally only 3 execution versions are retained per plugin; live
leases keep older versions until a subsequent update can collect them. This is
execution-version retention, not an extra configuration/database backup scheme.

Trusted service origins are an operator trust boundary, not an Internet sandbox.
An allowed HTTPS hostname must be controlled by your deployment; use network
policy to restrict egress and prevent DNS/endpoint takeover.

## Independent declarative interceptors

```json
{
  "expected_revision":0,
  "interceptors":[
    {"id":"trace","enabled":true,"order":10,"plugin_ids":["echo"],
     "set_headers":{"x-demo":"from-miora"},"remove_headers":[]}
  ]
}
```

Chains are ordered by `(order,id)`; later sets win and later removes remove the
header. Empty `plugin_ids` applies to all channels. Built-in IDs use
`builtin-gpt`, `builtin-claude`, etc. The chain is snapshot-pinned with the
request, including the optimized native Responses route. Only non-sensitive
custom `x-*` headers are permitted. Authorization, cookie, key/token/secret,
forwarding/IP, internal attribution and transport headers cannot be modified.
No interceptor code execution, response rewriting, schema substitution or
provider impersonation is supported. Do not store secrets in custom values.

## Built-in adapters and current scope

Existing request preparation was moved into `plugins/builtin.rs`; the registry
explicitly registers the 7 primary built-in adapters shown in the console
(`OpenAI Compatible`, `Anthropic Claude`, `Google Gemini`, `Codex`,
`Antigravity`, `OpenRouter`, `Azure OpenAI`) alongside legacy compatibility
adapters (`Vertex`, `AWS`, `Cohere`, `Cloudflare`, etc.):

- **`openai` (`builtin-gpt`)**: Standard OpenAI-compatible Chat Completions,
  Responses, Embeddings, Images and Audio endpoints.
- **`claude` (`builtin-claude`)**: Native Anthropic `/v1/messages` with both API
  Key and Claude OAuth (`sk-ant-oat...` with automatic `anthropic-beta` headers
  and token refresh).
- **`gemini` (`builtin-gemini`)**: Native Gemini `generateContent` /
  `streamGenerateContent` with **Gemini 4 / 3.8** adaptation (automatically
  omits unsupported `temperature`/`top_p` on Gemini 4, defaults `maxOutputTokens`
  to `65536`, and maps `thinking_level`/`reasoning_effort` to
  `thinkingConfig.thinkingLevel`).
- **`codex` (`builtin-codex`)**: OpenAI Codex Responses upstream with built-in
  OAuth 2.0 PKCE (`S256`) login and automatic token refresh.
- **`antigravity` (`builtin-antigravity`)**: Google Cloud Code Assist internal
  `v1internal:generateContent` / `streamGenerateContent?alt=sse` adapter with
  Google OAuth PKCE login, automatic `v1internal:loadCodeAssist` project ID
  discovery, envelope wrapping (`{ project, model, request }`) and response
  unwrapping.
- **`openrouter` (`builtin-openrouter`)** & **`azure` (`builtin-azure`)**:
  Dedicated OpenRouter and Azure OpenAI adapters.

Existing conversions,
OAuth helpers and billing behavior are reused rather than reimplemented. The
optimized native Responses path checks the registered built-in capability and
uses the same immutable interceptor snapshot; its mature streaming executor is
retained. External Responses channels intentionally fall back to the generic
registry executor with native passthrough output.

Built-ins are immutable binary-deployment plugins (`kind: builtin`,
`mutable: false`), not remotely updatable services. Disable a built-in **channel**
using channel controls. Their admin descriptors expose the common base URL/API
key fields; provider-specific advanced options still use the existing channel
configuration. Choose a built-in by its existing `engine`; choose a service
through `channel_plugin`. Legacy unknown engine aliases retain the historical
OpenAI-compatible built-in fallback. Remaining provider-specific response/OAuth
logic is a compatibility layer, not the extension mechanism for new services.

Legacy JSON-only services keep the JSON envelope. Multimodal services opt in to
the resource transport; native authentication proxies separately declare their
native routes and credential location. Only declared routes are accepted.
Independent remote services remain operator-managed; the managed Python ZIP
runtime supplies local process management instead. Neither mode attests remote
code or provides an operating-system sandbox.

## Verification

```sh
cargo test plugins::
python -m unittest discover -s examples/plugins/echo -v
python tests/http/verify_channel_plugins.py target/debug/uni-api-front
```

Use `.exe` on Windows. The HTTP test copies the binary to its temporary fixture,
starts an independent service and real gateway, authenticates admin APIs, and
checks Chat + Responses streams across update/disable, new-request versions,
rollback, failed health/config publication, model discovery, independent service
auth, ordered headers, secret redaction and restart persistence. No paid API,
external model or Python package install is needed.
