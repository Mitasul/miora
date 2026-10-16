# Administrator account and management API

Miora has a separate, password-authenticated administrator security domain. Gateway API keys (including a single configured key, an `admin`-looking name, or an actual admin-role Bearer key) **do not authenticate `/admin/api/*`**. The browser never stores a long-lived gateway token. Management code calls runtime/configuration methods directly; it does not replace a caller's Bearer token with a privileged one or proxy arbitrary paths.

## Configuration

| Environment variable | Default | Meaning |
|---|---|---|
| `MIORA_ADMIN_USERNAME` | `admin` | Exact, case-sensitive account name, 1–128 UTF-8 bytes, no control characters |
| `MIORA_ADMIN_PASSWORD` | unset | 12–1024 UTF-8 bytes; converted to a random-salt PBKDF2-HMAC-SHA256 verifier at startup |
| `MIORA_ADMIN_PASSWORD_HASH` | unset | Precomputed verifier, takes precedence over plaintext password |
| `MIORA_ADMIN_SECURE_COOKIE` | `true` | `true`/`1` or `false`/`0`; explicit trusted deployment setting, never inferred from forwarded headers |
| `MIORA_ADMIN_ORIGIN` | unset | Recommended fixed browser origin, e.g. `https://gateway.example.com`; HTTP(S) only, no credentials/path/query. Its scheme must agree with Secure-cookie configuration |
| `MIORA_ADMIN_SESSION_TTL_SECONDS` | `3600` | Absolute lifetime, 300–86400 seconds; no sliding renewal |
| `MIORA_ADMIN_TRUSTED_PROXIES` | empty | Comma-separated exact IPv4/IPv6 addresses permitted to append trustworthy X-Forwarded-For hops for login limiting; no CIDRs or hostnames |

When neither password variable is set, login fails closed with `503 admin_not_configured`. There is **no default password**. Invalid hash, weak/empty plaintext password, malformed origin, or invalid security settings fail application initialization. A configured hash cannot silently fall back to plaintext.

For local HTTP development only:

```powershell
$env:MIORA_ADMIN_USERNAME = 'admin'
$env:MIORA_ADMIN_PASSWORD = '<a unique strong password of at least 12 bytes>'
$env:MIORA_ADMIN_SECURE_COOKIE = 'false'
$env:MIORA_ADMIN_ORIGIN = 'http://127.0.0.1:8080'
```

Use HTTPS and keep Secure cookies enabled in production. Configure the reverse proxy to preserve the public Host header, or set `MIORA_ADMIN_ORIGIN` explicitly. `X-Forwarded-Proto`, `X-Forwarded-Host` and `Forwarded` never control cookie security or Origin checks. Login throttling ignores `X-Forwarded-For` unless the real TCP peer is explicitly listed in `MIORA_ADMIN_TRUSTED_PROXIES`. For a trusted peer, a single valid XFF header (at most 16 IPs / 1024 bytes) is traversed right-to-left through trusted hops, stopping at the first untrusted client address; malformed or duplicate headers fall back to the peer. Configure trusted proxies to overwrite or correctly append XFF. Without this trusted-proxy setting, all clients behind one reverse proxy share its rate-limit bucket and an attacker can consume that shared budget; configure trusted client attribution plus per-client limits at the edge for public reverse-proxy deployments.

### Offline password hash

Format:

```text
pbkdf2-sha256$600000$<hex salt>$<hex derived key>
```

New plaintext credentials use 600,000 iterations, a random 32-byte salt and a 32-byte verifier. Loaded hashes accept 100,000–2,000,000 iterations, 16–64 salt bytes and exactly 32 verifier bytes. Generate a verifier without putting the password in shell arguments:

```python
import getpass, hashlib, secrets
salt = secrets.token_bytes(32)
password = getpass.getpass('Administrator password: ').encode()
assert 12 <= len(password) <= 1024
key = hashlib.pbkdf2_hmac('sha256', password, salt, 600_000, 32)
print(f'pbkdf2-sha256$600000${salt.hex()}${key.hex()}')
```

Treat the verifier as sensitive configuration. Environment rotation takes effect on restart; restart also invalidates all sessions.

## Browser/session contract

All responses are JSON with `Cache-Control: no-store`. Errors use:

```json
{"error":{"code":"authentication_required","message":"Administrator login required"}}
```

Login:

```http
POST /admin/api/login
Content-Type: application/json

{"username":"admin","password":"..."}
```

Successful login returns this shape (no session bearer token in JSON):

```json
{"user":{"username":"admin","role":"admin"},"csrf_token":"random-token","expires_at":1890000000}
```

The session cookie is `miora_admin_session`, with `Path=/admin; HttpOnly; SameSite=Strict; Max-Age=<TTL>` and normally `Secure`. It has no Domain attribute. Tokens and CSRF values each contain 256 random bits; the session table stores SHA-256 hashes of cookie tokens, not the raw token. Login rotates the existing session; logout invalidates server-side state and clears the cookie.

* `GET /admin/api/session`: returns the same user/CSRF/expiry shape, or 401.
* `POST /admin/api/logout`: requires the session and `X-CSRF-Token`; returns `{"ok":true}`.
* Every non-GET/HEAD protected request requires the matching CSRF token and an accepted Origin/fetch-site. CLI clients may omit Origin, but not CSRF.
* Mutation bodies use `Content-Type: application/json`, except the exact `POST /admin/api/plugins/upload` route, which requires `multipart/form-data`. The upload still requires the same session, origin and CSRF checks. Keep CSRF in application memory; refresh it with `/session` after navigation. Do not use localStorage/sessionStorage for passwords, gateway keys, or session tokens.
* Cross-origin admin CORS is not enabled, including the legacy permissive gateway CORS path. Cross-site/same-site-but-not-same-origin browser mutations are rejected.
* Requests have a ten-second body-upload limit. Login bodies are at most 16 KiB, normal mutation bodies at most 256 KiB, and complete plugin multipart bodies at most 512 KiB with a 256 KiB file limit.

Authentication failures use 401; rejected CSRF/Origin uses 403; stale revisions use 409; bad bodies use 400/413/415. Login throttling returns 429 and `Retry-After: 60`. A source IP may make eight login attempts per minute. Password derivation runs in `spawn_blocking`, with at most four concurrent derivations and no mutex held over hashing. Sessions and rate-limit maps each have a 4,096-entry bound. Capacity pressure returns an error rather than evicting active sessions. There is no username-wide/global failed-login lockout that an attacker can use to lock out every administrator, although finite hash capacity and shared proxy-IP limits can still cause temporary denial of service.

## Management endpoints

All endpoints below require the account cookie. Mutations additionally require CSRF.

| Endpoint | Method | Contract |
|---|---|---|
| `/admin/api/overview` | GET | Runtime readiness, provider/model/key counts, 24h stats (null if unavailable), runtime resource details, supported capabilities |
| `/admin/api/channels` | GET | `{revision,channels,writable,secrets:"write_only"}`; provider name, sanitized base URL, engine, models, API-key count, temporary flag |
| `/admin/api/channels/detail?provider=NAME` | GET | Source-coherent `{provider,revision,base,effective,schema}`; write-only credentials are masked, not plaintext. Use this detail revision when editing |
| `/admin/api/channels/models` | POST | `{provider?,channel:{engine?,base_url?,api?,channel_plugin?,plugin_config?}}` queries available models without saving; returns `{models:[{id,name?}],source:"upstream"\|"plugin"\|"catalog"}` |
| `/admin/api/channels/parameter-schema` | POST | Channel draft plus optional request context; returns actual protocol `capabilities`, source `revision`, and whether the channel draft is `saved` |
| `/admin/api/channels/preview` | POST | Same input; pure production request preparation, with redacted headers/body/URL and value-free policy trace; no upstream invocation |
| `/admin/api/channels` | POST | `{revision,channel:{provider,base_url,engine,api,model:[...]}}` creates a channel in a local configuration source |
| `/admin/api/channels` | POST | `{revision,copy_from:"existing-name",channel:{provider:"new-name"}}` copies the saved source document server-side, including credentials, aliases, plugin settings and model prices; only the new name is supplied, no secrets are returned |
| `/admin/api/channels` | PATCH | `{revision,provider,patch:{...}}`; omitted fields/secrets are preserved; provider renaming is not supported |
| `/admin/api/channels` | DELETE | `{revision,provider}` removes a configured provider |
| `/admin/api/api-keys` | GET | `{revision,api_keys:[{id,position,name,groups,enabled,role,models,rate_limit,masked_key}],writable,read_only}`; includes disabled source entries, never key plaintext |
| `/admin/api/api-keys` | POST | `{revision,models:["all"],role:"user",name,groups:["default"],enabled:true,rate_limit:"60/min"}`; optional metadata, random server-generated key; returns `{ok,revision,id,api_key,show_once:true}` |
| `/admin/api/api-keys` | PATCH | `{revision,id,...}`; updates only supplied name, groups, enabled, models, role or rate_limit; disabled keys stay in the source but cannot authenticate |
| `/admin/api/api-keys` | DELETE | `{revision,id}`; removes the key and refreshes runtime authorization |
| `/admin/api/usage?days=30&model=NAME&channel=NAME` | GET | One filtered report: `summary`, per-model `usage`, per-channel `channels`, and `trend` with Unix-second `bucket_start`; `from`/`to` are inclusive/exclusive seconds, `bucket_seconds` is 3600 for one day and 86400 for longer ranges (UTC). Every group retains reported token/cache counts, coverage, success/failure, latency and snapshot-based USD estimates; unknown facts are null, channel/model lists have separate truncation flags, and no raw key is exposed |
| `/admin/api/logs` | GET | Actual in-process admin audit ring, **not request logs**: `{scope:"admin_audit",logs:[{timestamp,action,outcome}],persistent:false,capacity:500,request_logs_available:false}` |
| `/admin/api/backend-logs` | GET | `after`, `level`, `q`, `limit` (1–500); incremental runtime events, bounded 2000-entry in-memory ring, not request bodies or persistent stdout capture |
| `/admin/api/workspace` | GET | `path` selects a directory or text file; empty path lists virtual roots. Raw local `config.yaml` contains secrets; remote source view is redacted and read-only. Docs/examples/plugins are read-only; runtime/plugin views are virtual files |
| `/admin/api/workspace` | PUT | `{path:"config.yaml",content,revision}`; raw source replacement through existing compiler/publisher, preserving text and comments, up to 1 MiB; no backup or content digest |
| `/admin/api/test/models` | GET | `position` (1-based source key position), `endpoint`; returns allowed channel/model pairs and whether exact targeting is available; no plaintext credential |
| `/admin/api/test` | POST | `{position,endpoint,provider?,request:{model,stream:false,...}}`; actual in-process gateway inference, 180-second timeout and 4 MiB response cap; returns actual HTTP status, elapsed time, Request ID and response. Exact targeting requires an admin API key and Responses |
| `/admin/api/settings?provider=NAME` | GET | Persistent system settings by default (`__uni_console_global_preferences__`); `writable` and `persistence` describe source access. An explicit channel provider retains its runtime settings view |
| `/admin/api/settings/schema` | GET | Runtime channel-settings schema |
| `/admin/api/settings/validate` | POST | Validate a revisioned settings mutation without applying it |
| `/admin/api/settings` | PATCH | Validate, atomically save global preferences to the local source and hot-apply them |
| `/admin/api/plugins*`, `/admin/api/interceptors*` | delegated | Session/CSRF/body-limited plugin management; see plugin documentation for exact route and schema support |

Overview `capabilities` truthfully exposes `channels_write`, `api_keys_write`, `settings_write`, `request_logs:false`, `admin_audit:true`, and `plugins:true`. Persistent writes are unavailable for remote configuration sources. Missing functionality must be shown as unavailable/read-only; do not display fake save buttons.

### API-key lifecycle

Keys are generated using 256 random bits, never from timestamps or process IDs. Creation defaults to the ordinary `user` role and `["all"]` model rules. The only accepted explicit roles are exactly `user` and `admin`; selecting admin is a deliberate promotion for legacy gateway management, **not permission to log into the account backend**. An explicitly empty model list is rejected (the legacy compiler interprets empty rules as all models); revoke a key instead of attempting to deny access with an empty list. Models and roles can be changed by id; deletion revokes the key after runtime refresh (already-dispatched upstream requests are not cancelled). Return the plaintext key only in the successful POST response and keep this one-time display in memory; closing the dialog loses it. The list, logs and other reads never reveal it. This API does not allow supplying a chosen token, silently renaming/rotating an existing token, or writing arbitrary key preferences. Use POST then DELETE for rotation.

Both key and channel writes use a source-file revision and the same admin write gate, then the publisher's shared writer mutex/CAS; get a fresh list after every change.

The local source revision uses file modification time and length, not a content checksum. This is an optimistic save-conflict token, not tamper detection. A failed publication restores the source contents but may change this revision; refresh before retrying. Password verification and session authentication remain unchanged.

### Model discovery

`POST /admin/api/channels/models` uses the normal administrator session, JSON and `X-CSRF-Token` requirements, but does not require a configuration revision because it never saves a channel. For a new channel, send its engine, address and credentials. For an existing channel, send `provider` and only the edited fields:

```json
{"provider":"my-channel","channel":{"engine":"gpt"}}
```

The server reads the actual saved local source or remote runtime configuration before merging the draft. Missing or empty `api` reuses the saved supplier key; omitting `base_url` preserves the original address including secret query parameters. Do not submit masked credentials or a sanitized URL as an unchanged value. The response contains model ids/optional names and the source label, not credentials or the full upstream URL.

* `gpt`, `claude`, `gemini` and `openrouter`: fetch the supplier's model-directory endpoint, with the existing 20-second request timeout, and return `source:"upstream"`. Only model-list GET requests are made; no generation request is issued.
* Service plugins: call the registered plugin's model-discovery API and return `source:"plugin"`. Registry defaults, saved `plugin_config`, and submitted schema fields are merged. For legacy channels storing the supplier key in top-level `api`, its first key is mapped to `manifest.auth.config_key` with the same precedence as real plugin execution; the internal placeholder `__channel_plugin__` is not a credential.
* Codex: return the project's bundled model catalog with `source:"catalog"`. This is explicitly not a successful upstream catalog query.
* Azure: return `model_discovery_unsupported`; enter deployment names manually instead of choosing from an invented catalog.

Discovery failure returns an actionable error without the supplier URL, query key or credential. Users may still enter model ids manually. Fetching choices does not automatically select them or persist them; save the selected `model` list through normal channel creation/update.

### Parameter schema and offline request preview

The channel editor's Basic, Models and Parameters pages edit one channel document. The following two authenticated POST endpoints accept the same shape (normal JSON/CSRF requirements apply):

```json
{
  "provider":"optional-saved-channel-name",
  "channel":{"preferences":{"parameter_policy":{"channel":{"set":{"temperature":0}}}}},
  "request":{
    "endpoint":"/v1/chat/completions",
    "model":"public-alias",
    "stream":false,
    "body":{"messages":[{"role":"user","content":"Preview input"}]}
  }
}
```

`channel` is a partial source draft, not a separate replacement config. Missing source fields are reused server-side, including credentials and the original base URL/query. The `request` object is optional; the default endpoint is `/v1/chat/completions`. When no model is given the first configured public model is selected, and when no body is supplied a small local sample is used. Neither endpoint changes the source file, runtime state, credential scheduler or plugin registration.

`POST /admin/api/channels/parameter-schema` returns:

```json
{"capabilities":{"version":1,"endpoint":"/v1/chat/completions","endpoints":["/v1/chat/completions","/v1/responses"],"profile":"chat","scope":"supplier_wire","body_supported":true,"header_supported":true,"generation":{"temperature":{"supported":true,"path":"/temperature","type":"number","minimum":0,"maximum":2,"nullable":true}},"fields":[],"system_prompt":{"supported":true,"path":"/messages","modes":["inherit","prepend","append","replace","remove"]}},"revision":"source-revision","saved":false}
```

The example is abbreviated. Capabilities come from the same parameter module used by production preparation, not a UI-maintained vendor list. `generation` is a name-to-descriptor map, and root `fields` is its equivalent array with names. Descriptor paths are **final upstream JSON Pointers**, so Gemini output limits target `/generationConfig/maxOutputTokens`, not a parallel OpenAI token field. Respect `supported`, type/range/nullable metadata and the selected endpoint; this describes protocol shape, not a guarantee that every supplier model supports every optional field. `native_preview` identifies the dedicated native Responses path and is not a general preview enable/disable flag. Service endpoint choices are intersected with the registered manifest and parameter-core support; an empty list means unavailable, not unrestricted.

`POST /admin/api/channels/preview` returns:

```json
{
  "scope":"upstream_request",
  "method":"POST",
  "url":"https://supplier.example/v1/chat/completions",
  "headers":{"authorization":"********","content-type":"application/json"},
  "body":{"model":"supplier-model","messages":[],"temperature":0},
  "requested_model":"public-alias",
  "upstream_model":"supplier-model",
  "policy_trace":[{"path":"/temperature","source":"channel","action":"set"}],
  "policy_report":{"configured":true,"trace":[],"warnings":[]},
  "capabilities":{},
  "revision":"source-revision",
  "saved":false
}
```

This example is also abbreviated. `saved:true` means the merged channel draft equals its saved source document; it does **not** mean the preview saved anything. The revision belongs to that source read. The response uses the actual production `PreparedAttempt` method, endpoint, body, wire model and policy report. Native Responses uses its production native payload/header builders rather than a generic substitute. No HTTP send or OAuth resolution/refresh occurs; any dispatch-time authentication limitation is shown by the production report. Unsupported exact preparation returns `422 preview_unsupported`, never a fabricated successful wire request.

For a service plugin, `scope` is `plugin_envelope`: the body is the host's actual `/execute` envelope, **not the final supplier wire request inside that service**. Header policies cannot manipulate service control authentication. The registry is pinned for capabilities/preparation, and the envelope's `channel.config` is structurally redacted using historical secret classification plus the pinned manifest, including non-string secrets. All preview URLs remove credentials/query/fragment; recognized provider keys and secret header values are hidden. Reports contain only paths, sources, actions and warnings, never parameter values or credentials. Request payloads are not included in administrator audit entries.

### Saving parameter policies and returning to inheritance

Policies are stored under `preferences.parameter_policy` and validated by the production `validate_policy` before the existing revision/CAS publisher writes them. Publication refreshes live configuration without a service restart. A channel without an explicitly configured policy retains its previous behavior.

```json
{
  "defaults":{"temperature":0.7},
  "channel":{"set":{"top_p":0.9},"remove":["/metadata/obsolete"]},
  "models":[{"match":{"requested_model":"public-alias"},"set":{"temperature":0},"remove":[]}],
  "headers":[{"name":"X-Example","mode":"set","enabled":true,"secret":false,"value":""}],
  "system_prompt":{"mode":"append","value":"Additional system instruction"}
}
```

`defaults`/`set` are wire JSON objects, not maps of pointer strings; `remove` is an array of JSON Pointers. Runtime ordering, final constraints and system-prompt conversion belong solely to the shared production policy module, which preview also calls. In particular, do not add multiple protocol-specific token-budget fields to one draft.

Ordinary channel source fields are deeply merged, preserving existing/unknown extension fields. Parameter policy has explicit replacement semantics so selecting **inherit** really removes an override:

* Omitting `preferences.parameter_policy` entirely preserves it.
* Supplying that object replaces its known semantic sections. Missing known sections are cleared; unknown policy metadata remains. Explicit `defaults` and scope `set` objects replace their old maps, so deleting a key or sending `set:{}` clears its old override. `remove`, model-rule and header arrays replace their previous lists; removed rows do not reappear. Unknown metadata on retained sections/matching rows is preserved.
* `0`, `false`, `""` and `null` remain explicit values, not omission. The shared validator/capabilities decide whether a particular field accepts them.
* Header names must be unique case-insensitively, including disabled rows. Header rows match saved names case-insensitively. Missing `value`, `***` or `********` in retained set-mode rows reuses the saved value. `value:""` is explicitly empty. `inherit`/`remove` modes require no value and discard it. A new set-mode row without a usable value returns `400 invalid_parameter_policy`. If an older source has duplicate names, omitted/masked credentials are ambiguous and require an explicit value rather than selecting the first old row.
* Authorization/cookie/API-key/token/secret-like headers are always masked, even with `secret:false`; arbitrary headers marked `secret:true` are also masked. The `secret` boolean remains a boolean in source views.
* Masked values inside advanced JSON/unknown metadata/defaults/set are resolved only against the same saved location; a placeholder without a saved value is rejected. Multiple old rules for the same model cannot silently select a credential for a masked draft: provide explicit values or consolidate those rules. Placeholders are never silently written or sent upstream.

Editing the advanced JSON starts from the original full policy document rather than reconstructing only visible form fields. No OAuth-management, request-log or unrelated-page placeholder API is implied by these two endpoints.

### Persistent channel changes

Get `revision` from `/channels` or, when editing, the matching `/channels/detail`, not `/settings`. It uses source file modification time and length, not a content digest. Writable channel/key projections and their revisions come from the **same source read**, with local pure compilation and no discovery/network calls; they are not mixed with a separately refreshed runtime snapshot. Runtime-only overlays do not appear in the writable source view. Edits are serialized by a separate, nonblocking config-write lock; a concurrent admin write returns 409 instead of queueing behind authentication. A stale revision is rejected. Remote `CONFIG_URL` sources are read-only and return 403 on write.

Creation requires an explicit nonempty `model` array, without `all`, to avoid newly introduced implicit discovery/probes. Standard channels require an HTTP(S) `base_url` without URL credentials; channels declaring `channel_plugin` may omit it. Plugin fields `channel_plugin` and `plugin_config` are validated against the enabled registry manifest. Detail views use manifest-aware secret masking, including unknown-plugin fail-closed masking. PATCH merges plugin config values and treats a `***`/`********` placeholder only as 'preserve an already existing field', never as a new credential. Other recognized masked placeholders are restored only from the same saved location; a placeholder with no saved value is rejected. Omit unchanged ordinary secrets when possible. Persistent channel changes validate and publish through `RuntimeConfigPublisher::apply_patch_revision`, which coordinates all in-process writers and checks the source revision. Refresh after any failed write before retrying.

### Virtual models, accounts, and streaming tests

All routes retain admin session authentication and CSRF for writes:

| Route | Purpose |
| --- | --- |
| `GET /admin/api/virtual-models` | Source revision, complete virtual definitions, and channel model names |
| `PUT /admin/api/virtual-models` | `{revision, virtual_models}`; replaces the complete virtual definition object, including deletions |
| `GET /admin/api/channels/keys?provider=NAME` | Masked per-key enabled/cooldown/failure/Codex status; no raw credentials |
| `POST /admin/api/channels/keys` | `{revision, provider, import}`; append text/JSON credentials or Codex auth data |
| `PATCH /admin/api/channels/keys` | `{revision, provider, position, enabled}` or `{revision, provider, position, refresh:true}` |
| `POST /admin/api/test/stream` | Real Chat/Responses inference as SSE; saved gateway credentials stay server-side |

Channel PATCH also accepts `{revision, providers:["first","second"], patch:{enabled:false}}`; batch fields are limited to `enabled` and `groups`, with a single publication. `enabled:false` removes a channel from runtime routing without deleting its source row. Per-key positions are one-based source positions; persistent disablement is stored as `disabled_key_positions`.

Codex, Claude, and Antigravity refresh performs real network authorization, not a local status simulation. Claude and Antigravity browser logins retain refresh credentials when returned by the provider and use their provider-specific token endpoints; rotated refresh tokens survive restarts. Accounts previously saved as access tokens only must sign in again. Cross-provider quota windows are not implemented. See [gateway configuration and limitations](gateway-features.md) for BYOK authentication, virtual routing, response policies, and playground behavior.

The publisher's shared writer lock coordinates admin and legacy configuration writes within one process. Filesystem edits by unrelated processes are not transactionally locked; use a **single writing instance** and do not concurrently edit the YAML externally. Runtime-only imported channels are not persistent source rows and cannot be edited/deleted with this API.

### Per-model pricing

Open a channel's **Models** tab and expand **Price** for the model to set its own input, output, cache-read, five-minute cache-write and one-hour cache-write USD rates per million tokens. Prices belong to the channel and public model name (alias when present), not to a global model registry. Zero is free; missing or explicitly cleared required prices do not produce a fabricated cost estimate. Existing channel-wide defaults remain compatible and are displayed in each model's form.

Store these rates under `preferences.usage_pricing.models`, for example `{"models":{"public-model-a":{"input":2,"output":8,"cache_read":0.2},"public-model-b":{"input":4,"output":16,"cache_read":0.4}}}`. In channel drafts, an explicitly supplied `usage_pricing.models` replaces the complete price catalog so renamed/deleted models cannot leave stale entries; omitting it preserves the catalog. The UI preserves unrelated saved entries. Other channel preferences retain their normal merge behavior. Model aliases carry their prices when renamed. Each request snapshots only the actual channel/model's applicable rates; changing prices never recalculates historical costs.

### Persistent system settings

Use the revision and schema from the settings view:

```json
{
  "revision":"revision-from-settings-view",
  "operation_id":"a-new-random-UUID",
  "changes":[{
    "provider":"__uni_console_global_preferences__",
    "set":{"/preferences/model_timeout":60},
    "remove":[],
    "reset":false
  }]
}
```

Global settings responses include `persistence:"source_file"`. Validation-only requests never write files or mutate the runtime. A save validates the entire candidate, checks both source and control revisions, then uses the same serialized, atomic source/snapshot publication transaction as channel edits. Publication failures restore the original source. Existing effective global overrides are materialized and cleared only after publication, so they cannot mask the saved values. Other preferences, channel credentials and environment placeholders are preserved. Successful saves apply immediately and survive process restarts, even with the database disabled. Operation IDs remain idempotent within the running instance; after a restart, fetch a fresh revision before editing.

Remote `CONFIG_URL` sources return `writable:false,persistence:"read_only"`; global validation/save attempts return `403 configuration_read_only`. The UI disables editing instead of pretending to persist remote settings. Explicit non-global channel settings still use the legacy runtime-only mutation contract; mixed global/channel batches are rejected. This admin API does not expose plaintext `/secrets`, raw `/export`, generic configuration replacement, credit mutation, or arbitrary `/v1` forwarding.

Run `python tests/http/verify_settings_persistence.py target/debug/uni-api-front.exe` for real save/restart, validation-only, revision/idempotency, rollback, secret preservation and remote read-only checks with database enabled and disabled.

## Integration contract

The parent application wires only these pieces:

1. Declare `mod admin;` in `src/lib.rs`.
2. Add `pub(crate) admin: crate::admin::AdminState` to cloneable `AppState`, initialized using `AdminState::from_env()?`.
3. Before legacy CORS preflight, decompression and proxy dispatch: if `crate::admin::supports(request.uri().path())`, return `crate::admin::handle(&state, request).await`.
4. Start Axum with `into_make_service_with_connect_info::<std::net::SocketAddr>()` for real peer-address throttling. Without connection metadata, requests deliberately share a conservative `unknown-peer` bucket. Only configured trusted proxy peers may supply client IP attribution.
5. Serve the static `/admin` and login/assets routes separately. `state.admin.is_authenticated(request.headers())` is a synchronous session check for the HTML redirect gate; API endpoints always perform their own authorization.
6. Plugin delegation expects `crate::plugins::api::handle(state.plugins.as_ref(), request)`. Do not route plugin management around this session/CSRF gate.
7. Existing `/v1` authorization must independently require the exact administrator role where appropriate; single-key/first-key heuristics and substring checks must not confer administrator rights.

## Verification and limitations

Unit tests cover verifier rejection/matching, random session rotation and logout, duplicate-cookie rejection, expiration, CSRF mismatch, per-client rate isolation, forged forwarded origin, body limits, credential redaction and channel validation. Run with the repository's Rust toolchain:

```text
cargo test admin::
```

For a fresh installation, `MIORA_BOOTSTRAP_EMPTY_CONFIG=true` explicitly creates an empty local source file when none exists, only if valid administrator credentials are configured. It does not create any inference key/provider, does not overwrite an existing file, and cannot be combined with remote `CONFIG_URL`. The administrator can then create channels and API keys through the backend.

For an automatically isolated regression (also compatible with `scripts/verify-http.py`):

```powershell
python tests/http/verify_admin.py target/debug/uni-api-front.exe
```

This starts and cleans up disposable gateway instances to verify empty bootstrap, channel/key CRUD, disabled credentials, and hash-only credentials. No real upstream is called.

For a running disposable instance with a valid runtime configuration:

```powershell
# Use the same plaintext password as the server, including when it loads a hash.
python tests/http/verify_admin.py --base-url http://127.0.0.1:8080 --exercise-writes
```

Set `MIORA_TEST_USER_BEARER` to a configured non-admin key to verify it cannot access the account backend or leak through read responses. `--expect-disabled` tests fail-closed unconfigured login. `--skip-rate-limit` is useful when repeated tests share one address; otherwise the last test intentionally consumes the login budget. Full writes briefly create/update/delete an explicitly modelled `example.invalid` test channel and a random ordinary API key; run this only against a disposable configuration, not production.

Sessions, rate limits and the bounded audit ring are process-local: restart invalidates them, multiple replicas require sticky sessions, and rate limiting is not distributed. Audit entries contain only timestamps, fixed action names and outcomes, never passwords, request bodies, cookie values or raw errors. There is no MFA, account recovery UI, multi-user administration, durable audit sink or request-detail log API in this module. The environment/configuration operator remains the root of trust. Admin channel creation and plugin capabilities may intentionally reach configured upstream endpoints; perimeter egress policy is still required if administrators are not fully trusted.
