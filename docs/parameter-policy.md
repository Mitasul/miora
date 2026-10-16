# Channel parameter policy

This phase adds an independently implemented parameter policy to the existing
request builders. It does not replace provider adapters or introduce a second
preview-only merger. The policy is stored at
`providers[].preferences.parameter_policy`.

Production preparation runs body policy validation before applying headers, using
the resulting report in both stages. The header stage does not repeat the full
schema walk. Native headers are converted only when enabled header rules exist;
an absent interceptor chain likewise leaves the native header map untouched.

## Schema

```json
{
  "defaults": {
    "temperature": 0.7,
    "max_tokens": 512,
    "metadata": {"client_tag": "default"}
  },
  "channel": {
    "set": {"top_p": 0.9, "metadata": {"channel_tag": "configured"}},
    "remove": ["/metadata/obsolete"]
  },
  "models": [
    {
      "match": {"requested_model": "public-chat"},
      "set": {"max_tokens": 1024},
      "remove": ["/top_p"]
    }
  ],
  "headers": [
    {"name":"X-Application","mode":"set","value":"miora","enabled":true,"secret":false},
    {"name":"X-Private-Label","mode":"set","value":"private-value","enabled":true,"secret":true},
    {"name":"X-Obsolete","mode":"remove","enabled":true}
  ],
  "system_prompt": {"mode":"prepend","value":"Answer concisely and explain assumptions."},
  "future_extension": {"preserved_but_not_executed":true}
}
```

`defaults` and `set` contain **wire-body fields**, not a universal bag of all
vendors' parameter names. For example, Claude uses `max_tokens`, Responses uses
`max_output_tokens`, and Gemini uses `generationConfig.maxOutputTokens`.
Use the returned capability descriptor's JSON pointer to construct the right
nested object. Do not send all budget aliases at once.

Known sections are validated. Unknown policy extension fields are preserved in
the stored JSON but are not executed. Within supported wire bodies, advanced
unknown fields can still be explicitly set; doing so is not a declaration that
the particular supplier/model supports them.

### Values, inheritance and deletion

- `0`, `false`, `""` and `null` are explicit JSON values. They never mean delete.
- An omitted policy assignment inherits the previous request-building result.
- `remove` is an array of RFC 6901 JSON pointers. `~0` escapes `~` and `~1`
  escapes `/`; array elements can be removed by index.
- Object assignments merge recursively. Arrays replace the previous array;
  they are not concatenated. Explicit scalar/null assignments replace a value.
- `defaults` only fills leaves absent from the **original client's presence
  information**, not just absent from the already-constructed wire JSON.
  Existing client null/false/zero/empty values block defaults.
- An adapter-generated default is not a client value: a configured Claude/Gemini
  output default can replace the adapter's generated token budget.
- A default that is blocked by client presence never creates a null placeholder
  or resurrects a field deliberately removed by the legacy compatibility layer.
- Numeric and protocol types still matter. For example, a zero or null output
  budget is a value, but is rejected because this phase supports positive integer
  output limits. Use `remove` to delete an optional field; Claude's required
  `max_tokens` cannot be removed.

Header modes are independent `inherit`, `set`, and `remove`; empty string is a
valid `set` value. Names are case-insensitively unique across the entire policy,
including disabled rows. Duplicate names are rejected rather than ambiguously
restoring a secret from the wrong row. A header `set` requires an explicit
string after the admin layer has restored a deliberately omitted/masked existing
secret. An omitted value is not the same as an explicit empty string.

The admin form uses known-section replacement semantics when saving a policy so
clearing an override really restores inheritance; it preserves unknown extension
fields rather than silently dropping them. Persistence/secret restoration is
owned by the authenticated admin API, not by the request parameter engine.

## Fixed production order

1. Existing protocol construction; retain client input and pre-legacy wire
   provenance when a policy is configured.
2. Existing `post_body_parameter_overrides` compatibility behavior.
3. New defaults, using client presence and mapped generation parameter families.
4. Channel `set`, then channel `remove`.
5. Channel-level `system_prompt` operation.
6. Matching model rules, in array order: each rule applies `set`, then `remove`.
7. The channel's ordered header entries and existing independent header
   interceptor chain in their normal preparation locations.
8. Final protocol constraints/signing in the existing adapter/executor.

Model matches are exact, case-sensitive **requested public model aliases**.
They are not substring/glob matches and do not match the upstream model name.
Multiple exact matches execute in their saved order. In particular a model rule
can override channel-level `instructions`/`messages`; the channel system prompt
is not reapplied afterwards to defeat the more specific rule.

Body/header phases touch separate structures. Final Codex restrictions and Azure
budget normalization run after the new body policy, and their changes are added
to the trace. Existing interceptor chains remain independent registries; policy
header entries do not redefine those chains. This phase has channel-level header
rules only, not model-specific header rules.

When `parameter_policy` is absent, the new layer returns without changing the
body or headers. The old overrides and legacy adapters retain their old behavior.
The native Responses executor selection predicate is shared with preview and
preserves the existing selection rules; this change does not broaden native
routing coverage or normalize saved engine names behind the operator's back.

## Supported profiles and scope

Capabilities describe **protocol field shapes**, not a promise about every model
or every deployment of that protocol.

| Wire profile | Temperature | Top-p | Output limit | Stop |
|---|---|---|---|---|
| Chat | `/temperature`, 0–2 | `/top_p`, 0–1 | `/max_tokens`; explicit Chat `/max_completion_tokens` alias supported | `/stop`, string or string array |
| Responses | `/temperature`, 0–2 | `/top_p`, 0–1 | `/max_output_tokens` | unsupported |
| Claude | `/temperature`, 0–1 | `/top_p`, 0–1 | required `/max_tokens` | `/stop_sequences`, string array |
| Gemini | `/generationConfig/temperature`, 0–2 | `/generationConfig/topP`, 0–1 | `/generationConfig/maxOutputTokens` | `/generationConfig/stopSequences`, string array |
| Codex Responses | unsupported | unsupported | unsupported by this policy phase | unsupported |

Codex retains its existing protocol requirements instead of pretending that all
Responses generation parameters work there. Reasoning effort, thinking and
service tier are not offered as supported policy controls in this phase.

Only one output-budget wire field may survive. A client's existing Chat
`max_completion_tokens` is not changed to deprecated `max_tokens` by an unrelated
policy. Presence is tracked across the budget alias family so a default under a
different spelling cannot override the client's supplied budget. Explicit
channel/model budget assignment can replace the earlier budget; conflicting
budget assignments in one section are an error. Cross-protocol mapping and
existing Azure GPT-5 normalization select the actual final wire field, not a
collection of token aliases.

Bodies for unsupported profiles and non-conversation endpoints (for example
embeddings, image, audio, video and search) are **not modified** by body policy.
Their report explicitly says `body_supported: false`, records a skipped stage,
and supplies a warning. Replay/multipart bodies cannot be rewritten by this JSON
policy. Supplier wire headers can still be controlled where supported.

For service plugins, the scope is `service_request_body`: policy affects only
`envelope.request.body` in the host-facing Chat/Responses protocol. It does not
claim knowledge of the plugin's internal supplier payload or its final wire
headers. Active `set`/`remove` header rules for a service channel are rejected;
the service control-channel Authorization is never overridden. Service endpoint
choices must also be intersected with the registered manifest. Legacy provider
body overrides were not applied to service envelopes before this feature, and
this phase does not silently introduce them there.

## System prompts

Modes: `inherit`, `prepend`, `append`, `replace`, `remove`.

- Chat: operates only on `role: system` messages. `developer` and ordinary
  messages retain their relative ordering. Prepend/append add a system entry
  before the first/after the last existing system entry; with none, the new
  entry is inserted at the beginning. Replace removes only system entries and
  inserts the configured value at their first position; remove removes them.
- Responses: operates on `instructions`, joining prepend/append text with a
  newline when both sides are nonempty. Native `input` items are not reordered.
  For Chat converted to Responses, an explicit prompt operation lifts only the
  converted Chat system entries into the instructions operation; developer and
  ordinary input entries are not removed or reordered.
- Claude: operates on `system`, preserving string or supported block-array form
  for prepend/append. Replace uses the supported string form; remove drops it.
- Gemini: operates on `systemInstruction.parts`; prepend/append add text parts,
  replace builds a new text instruction, remove drops the instruction.

Replacing with `""` is an explicit empty prompt, not deletion. Unsupported or
non-text prompt shapes return a preparation error instead of silently dropping
content. Existing protocol adapters retain their own cross-protocol message-role
conversion behavior; this module is not a replacement conversation converter.

## Headers and redaction

The policy acts on constructed supplier HTTP headers through real
`HeaderName`/`HeaderValue` parsing, not string concatenation. Invalid names,
CR/LF-bearing values and transport/internal attribution headers are rejected.
`inherit` leaves the existing prepared header untouched; it does not forward a
caller/admin header automatically.

Ordinary direct-provider authentication headers may be deliberately overridden
by an authenticated administrator, but are always sensitive in management
views. Authorization/auth, cookies, API keys, tokens and secret-like header names
are treated as sensitive regardless of `secret: false`. `secret: true` also
protects custom names. Dynamic Vertex/AWS authorization and AWS signing headers
are owned by their dispatch signer and cannot be overridden by these rules.

Preview does not refresh OAuth or run dynamic credential resolution. Its report
warns about dispatch-time authentication; it is not a guarantee that an OAuth
refresh/account selection will produce exactly the preview credential. The
admin response must also redact service envelope `channel.config` against the
plugin manifest, not just redact visible HTTP headers.

## Shared APIs for admin and runtime

```rust
validate_policy(policy: &Value) -> Result<(), String>
capabilities(provider: &Provider, endpoint: &str) -> Value
header_sensitive(name: &str, declared_secret: bool) -> bool
prepare_preview(
    registry: &PluginRegistry,
    provider: &Provider,
    requested_model: &str,
    upstream_model: &str,
    endpoint: &str,
    payload: Value,
) -> Result<PreparedAttempt, String>
```

`PreparedAttempt.policy_report` is serializable:

```json
{
  "configured":true,
  "profile":"chat",
  "scope":"supplier_wire",
  "body_supported":true,
  "header_supported":true,
  "trace":[
    {"path":"/temperature","source":"channel","action":"set"},
    {"path":"/top_p","source":"models[0]","action":"remove"}
  ],
  "warnings":[],
  "sensitive_headers":["authorization"]
}
```

Trace contains paths, sources and actions only—never configured secret values,
prompts or API keys. Sources can include `client`, `client_mapping`, `defaults`,
`channel`, `system_prompt`, `models[N]`, `headers[N]`,
`protocol_constraint`, and an unsupported-scope marker. Preview is not a durable
request-audit/retention feature.

Capabilities expose `profile`, `scope`, `body_supported`, `header_supported`,
`preview_supported`, `endpoints`, `generation` (a named descriptor map), `fields`
(the same descriptors with `name` for generic forms), `system_prompt`, and
unsupported controls. Each generation descriptor supplies `supported`, a JSON
pointer `path`, `type`, numeric range and `nullable`.

`native_preview` identifies use of the optimized native Responses builder; it is
**not** the overall preview availability flag. Ordinary Chat/Claude/Gemini
previews remain supported. Service capabilities from the provider-only function
have an empty `endpoints` list because that function has no registry argument;
the admin adapter supplements it from the registered manifest.

The preview delegates to the same registry prepare function for ordinary/service
routes and the same payload/header builders for native Responses. It performs
no network send, model discovery, key scheduling, OAuth refresh, billing or
plugin health check. The admin API is responsible for session/CSRF authorization,
draft construction, persistence CAS and redaction.

## Focused verification

The module has focused unit tests in `src/providers/parameters/tests.rs`, including:

- Client zero/false/empty/null presence, nested defaults and generated defaults.
- Deep merge, array replacement, escaped JSON pointers, exact model priority.
- Single budget fields, alias-presence regressions and Azure mapping.
- Claude required-budget removal, malformed rules and explicit constraint errors.
- Chat/Responses/Claude/Gemini prompt modes and model-over-channel prompt priority.
- Real header parsing, duplicate-name rejection, empty values and value-free traces.
- Unsupported endpoint skip, unchanged no-policy legacy behavior, final Codex constraints.
- Shared native preview selection including uppercase engine fallback and stream coercion.
- Service request-body scope and control-channel header rejection.

The parent integration workflow runs the limited policy/admin unit groups and
`tests/http/verify_parameter_policy.py`, comparing actual mock-upstream Chat and
native Responses request bodies with admin preview output. This phase does not
claim complete vendor/model compatibility, comprehensive OAuth preview parity,
new reasoning controls, or broad runtime/log-retention redesign.
