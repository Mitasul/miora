# Header-auth Chat: a real forwarding channel-service sample

SPDX-License-Identifier: Apache-2.0

These example files are original Miora repository code under its Apache-2.0
license. They do not incorporate code, components, tests, or documentation from
Zoaholic or another GPL reference project. No reference repository is needed.

## Why this sample exists

The `echo` example demonstrates the protocol with generated responses. This
sample instead makes **real outbound HTTP requests** and relays the upstream
response. It demonstrates one reason an independent adapter might be useful:
a Chat-compatible upstream authenticates with `X-Sample-Api-Key`, not the usual
bearer header.

Ordinary OpenAI-compatible suppliers should normally be **instances of the
existing generic channel**, not a new plugin for every vendor. This is a small
dynamic-extension example, not a recommended vendor-per-plugin architecture.

The only mocked model data is in `tests/http/verify_real_channel_plugin.py`, where
it is explicitly marked as a local test fixture. The plugin itself does not
fabricate completions, usage, a successful status, or an SSE `[DONE]` marker.
No actual vendor's production compatibility is claimed.

## Files and requirements

- `server.py`: authenticated v1 `/health`, `/manifest`, `/models`, `/execute`.
- `transport.py`: one upstream HTTP connection per request, bounded chunks and
  downstream-disconnect monitoring.
- `manifest.json`: the v1 registration manifest/configuration contract.

Python 3.10+ **standard library only**, no pip dependencies, Rust ABI, package
installer, generated SDK, or subprocess-per-request execution.

## Run a fixed-version service

PowerShell example for a local development listener:

```powershell
$env:HEADER_CHAT_SERVICE_TOKEN = '<independent service credential>'
python examples/plugins/header-chat/server.py --port 8766 --version 1.0.0
```

Keep it bound to `127.0.0.1` by default. The token is required; the example has no
built-in credential. `--token` is supported for disposable tests, but environment
configuration avoids placing a real credential in a process command line.

For a different implementation/deployment version, start a **separate process
and endpoint**:

```powershell
python examples/plugins/header-chat/server.py --port 8767 --version 2.0.0
```

The version is fixed for each process. `/health` reports its actual version and
**does not echo** `X-Miora-Plugin-Version`. `/models` and `/execute` reject
mismatched envelope versions. A registration declaring version `2.0.0` against
the `1.0.0` process therefore fails the host's health check and leaves the active
registration unchanged. The two sample processes run the same forwarding code;
they are deployment-isolation fixtures, not a claim that a CLI flag installs a
new code artifact.

## Configuration and registration

The host's startup trust policy must permit the service endpoint. For the local
HTTP example only, opt in to `MIORA_PLUGIN_ALLOW_LOOPBACK_HTTP=true` before
starting a **dedicated development gateway**. Do not change or restart a shared
preview/production instance merely to try this example.

Register through the existing authenticated admin API/UI. Copy `manifest.json`
and use this registration envelope, replacing all placeholders:

```json
{
  "manifest": "<the complete manifest.json object, not this string>",
  "endpoint": "http://127.0.0.1:8766",
  "service_auth": {
    "type": "bearer",
    "token": "<same independent service credential>"
  },
  "config": {
    "base_url": "https://your-header-auth-upstream.example/v1",
    "api_key": "<upstream X-Sample-Api-Key credential>",
    "timeout_seconds": 120
  },
  "expected_revision": 0
}
```

Send it to `POST /admin/api/plugins` using a verified admin session cookie and
`X-CSRF-Token`; never substitute an inference bearer for admin authentication.
The registered `manifest.version` must match the selected service process.
Updates use the current management revision as `expected_revision`.

`base_url` includes the API prefix (`/v1` in the example). The adapter adds
`/models` or `/chat/completions` to that path. It rejects URL credentials, query
strings and fragments. HTTP and HTTPS upstreams are supported; redirects are
returned as upstream statuses, **not followed**. HTTPS uses Python's normal
certificate verification.

Fields:

| Field | Type | Meaning |
|---|---|---|
| `base_url` | required string | Upstream API base URL |
| `api_key` | required secret string | Only sent as `X-Sample-Api-Key` to that upstream |
| `timeout_seconds` | integer, default 120 | Socket read/write timeout, 1–600 seconds; connect timeout is at most 10 seconds |

Registration defaults can be overridden by channel `plugin_config`. The host
may also fill the declared `auth.config_key` from a channel's scheduled upstream
key. These upstream credentials are separate from the service bearer and the
caller's inference/API/admin credentials.

Example channel with an exposed alias:

```yaml
providers:
  - provider: header-chat-channel
    channel_plugin: header-chat
    model:
      - upstream-request-model: public-alias
```

The host resolves `public-alias` to `upstream-request-model` before constructing
the execution envelope. The adapter forwards that request JSON without changing
its model or other fields. It does not replace the upstream's response `model`
with either name.

For a normal call, give the model a useful visible-output budget rather than a
16-token probe. For example, after configuring the channel and an inference key:

```powershell
$body = @{
  model = 'public-alias'
  messages = @(@{ role = 'user'; content = 'Explain TCP backpressure in three sentences.' })
  max_tokens = 512
  stream = $false
} | ConvertTo-Json -Depth 8
Invoke-RestMethod -Method Post -Uri 'http://127.0.0.1:8000/v1/chat/completions' `
  -Headers @{ Authorization = "Bearer $env:MIORA_INFERENCE_API_KEY" } `
  -ContentType 'application/json' -Body $body
```

For version 2, change registration `manifest.version` to `2.0.0` and `endpoint`
to the second process at port 8767. Keep the first process available while old
requests drain; a registry lease cannot keep a stopped remote process alive.

## Wire behavior

### Service ingress authentication

Every supported service endpoint verifies exactly one
`Authorization: Bearer <service token>` header. Even `/health` and `/manifest`
require it. A missing/wrong token fails before any upstream request.

Fresh upstream headers are constructed by the adapter:

- `X-Sample-Api-Key` from the declared secret config.
- `User-Agent: miora-header-chat/<actual process version>`.
- `Accept`, `Accept-Encoding: identity`, content type for JSON and transport
  headers created by `http.client`.

The service bearer, gateway caller bearer, cookies, CSRF tokens, arbitrary caller
headers and the whole plugin envelope are **not** forwarded to the supplier.
Logs do not print request bodies, prompts, upstream URLs or credentials.

### Model discovery

Host `POST /models` becomes a real upstream `GET <base_url>/models`. A successful
OpenAI-style `{ "data": [{"id": "...", "name": "..."}] }` catalog is converted
to the host's `{ "models": [...] }` format. If an upstream entry has no display
name, its ID is used as the name. No built-in/fallback model list is returned.

Discovery is bounded to 1 MiB and 4096 entries. Malformed success responses fail
with 502, not an invented catalog. Non-success upstream statuses/error bodies
are forwarded rather than changed to success.

### Execution

Only v1 JSON `POST /v1/chat/completions` envelopes are accepted. The adapter
actually sends `POST <base_url>/chat/completions` with the request body and
custom authentication header.

Nonstream JSON and stream SSE response **payload bytes** are forwarded unchanged,
including whitespace and UTF-8. The adapter does not parse chat completions or
SSE, infer usage, rename response models, synthesize a final event, or discard
valid tool-call responses with empty text. HTTP transport chunk boundaries may
change; payload bytes do not. A small safe response-header allowlist is used,
not a blind copy of hop-by-hop headers.

Upstream HTTP errors retain their status. The existing gateway can deliberately
sanitize plugin error bodies for security; direct calls to this service retain
the upstream error bytes. This sample does not change host behavior.

The adapter sends HTTP/1.1 chunked output. It writes the terminating **HTTP**
chunk only after clean upstream transport completion. A truncated HTTP response
closes the downstream connection without that terminator, never adds `[DONE]`,
and never starts a second JSON error response after SSE has already begun.
A clean HTTP EOF without a semantic SSE terminal marker remains exactly that;
the host/client decides whether it is a valid completion.

## Backpressure and cancellation

Chat forwarding reads at most 16 KiB per step and immediately writes it to the
client. There is no unbounded queue or whole-response buffer, including for
nonstream JSON. Blocking socket writes provide normal TCP backpressure. OS and
standard-library buffers still exist. The incoming JSON envelope is separately
bounded to 4 MiB; model discovery is intentionally buffered within its 1 MiB cap.

Each active forwarding request has a small watcher thread using `select` on the
client socket. After the full request body has been consumed, a client FIN/reset
causes immediate `shutdown(SHUT_RDWR)` of the owned upstream socket. This wakes
an upstream `getresponse()`/`read1()` even when the upstream sends **no further
bytes**. Cancellation does not merely wait for a future write to throw
`BrokenPipeError`. Cleanup closes the response and connection.

Cancellation is tested through the real gateway after the first semantic frame,
as well as directly against the service while the upstream is silent, both
before response headers and after the first frame. The test
requires the fixture upstream to observe socket closure within 2.5 seconds,
well below the configured 120-second timeout.

## Focused offline verification

Run only this new test; it starts its own temporary gateway/config/store, two
independent service processes and an explicitly mocked upstream fixture:

```powershell
python -B tests/http/verify_real_channel_plugin.py target/x86_64-pc-windows-gnu/debug/uni-api-front.exe
```

The test copies the binary into its temporary directory before execution. It
does not rebuild Rust, restart an existing preview, use real API keys, or call a
paid upstream. Assertions verify actual upstream **path, method, JSON body,
custom auth and request count**, so generated plugin-local fake responses cannot
pass. Coverage includes:

- Real model discovery, bad service auth and fixed actual health identity.
- JSON/SSE byte preservation, no-usage content and empty-text tool calls.
- Different public alias, upstream request model and upstream response model.
- Long JSON and a >2 MiB multi-chunk SSE stream; both chunked transfer and
  Content-Length JSON/SSE, including a truncated Content-Length failure.
- Midstream transport failure without a fabricated DONE.
- Active-stream and silent-upstream client cancellation, including direct service
  cancellation before upstream response headers.
- v1 in-flight stream while v2 becomes active on its separate process/endpoint.
- Wrong health version rejection without replacing the current version.
- Upstream failure statuses, direct raw error bytes and credential isolation.

## Limits, deliberately not implemented

- This is **not a code-package installer or a sandbox**. The operator trusts the
  service and its configured upstream destination with prompts and credentials.
  Use external egress controls; arbitrary untrusted `base_url` values are unsafe.
- No OAuth, multipart uploads, Responses protocol, embeddings, retries, automatic
  vendor discovery, distributed process management or durable work queues.
  Host scheduling/retries/logging/leases remain host responsibilities.
- No production vendor validation, complete tool-execution round trip, log
  retention feature, performance/RSS benchmark, TLS termination
  server or HTTP/2 server. Slow-consumer/RSS backpressure bounds and
  gateway cancellation before response headers are not tested in this scope.
  The example listener is plaintext HTTP for a trusted
  local hop; external deployment needs appropriate TLS/proxy/network controls.
- Python's threaded stdlib server is a small example, not production admission
  control. Two threads per active forwarding request require an external
  connection limit/supervisor for deployment.
- No HTTP pipelining or client write-half-close semantics: one complete request
  per connection, then server close. A FIN after the request is treated as
  cancellation, matching the gateway's normal full-disconnect behavior.
- Cancellation during DNS resolution or before a connected socket exists is not
  immediately interruptible by this stdlib approach. Connect attempts have a
  10-second socket timeout; DNS timing is platform-controlled. The verified
  cancellation guarantee covers established upstream sockets, including silent
  response bodies. HTTPS/WAN/proxy-specific cancellation is not validated here.
