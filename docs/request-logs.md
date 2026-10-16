# Request logs, retention and cleanup

Request logs are separate from administrator audit events (`/admin/api/logs`), aggregate usage/statistics, S3 facts and the request spool. The gateway does not change inference responses when logging is disabled, full, unsupported or unavailable. Capturing original request/response content is opt-in; defaults keep metadata only.

## Storage and startup

The parent initializes `AppState.request_logs` with `RequestLogs::from_env(!persistence.disabled())`. A failure to open the optional local log store is isolated using `unwrap_or_else(RequestLogs::unavailable)` rather than stopping an otherwise working gateway; this does not swallow core configuration or statistics-database errors. The unavailable instance reports `backend_enabled:false`, `status.worker_alive:false`, and `status.initialization_error`, and its storage operations return `503 request_logs_unavailable` (not a fake success).

* `MIORA_REQUEST_LOG_DIR`: defaults to `data/request-logs`.
* `index.sqlite`: a dedicated SQLite index/settings database (WAL mode).
* `bodies/`: independent captured-content files, never the inference request spool.
* `DISABLE_DATABASE=true`: a disabled stub; no request-log directory/database is created. Settings GET reports `backend_enabled:false`; persistence/query/mutation endpoints return `503 request_logs_disabled`.
* `MIORA_RUNTIME_LOG_MANAGED=1`: declares that a local supervisor really manages runtime stdout/stderr rotation. Otherwise the API omits editable runtime rotation settings and rejects attempts to change them. This does not claim to control container/docker/systemd log drivers.

Use a distinct directory per gateway instance. The directory/index are designed for one owning process, not simultaneous independent writers sharing a directory. On startup, leftover active rows become incomplete (`process_restarted`) and orphan body files are removed. This does not imply that old clients completed successfully.

The settings table is exactly:

```sql
log_settings(id INTEGER PRIMARY KEY CHECK(id=1), revision INTEGER, config_json TEXT)
```

Row `id=1` stores an integer CAS revision and the **settings object itself** in `config_json`, not an extra `settings` wrapper. A runtime supervisor reads `config_json.runtime.file_max_bytes` and `config_json.runtime.retained_files`. `retained_files` includes the current file, not just backups.

## Settings

`GET /admin/api/log-settings` returns:

```json
{
  "revision":1,
  "backend_enabled":true,
  "runtime_rotation_managed":true,
  "settings":{
    "enabled":true,
    "capture_fields":[],
    "body_retention_hours":0,
    "row_retention":{"mode":"manual","days":30},
    "limits":{"per_request_bytes":1048576,"total_body_bytes":268435456},
    "runtime":{"file_max_bytes":10485760,"retained_files":3}
  },
  "capture_fields":["request_headers","request_body","upstream_request_headers","upstream_request_body","upstream_response_headers","upstream_response_body","response_headers","response_body"],
  "status":{"active_records":0,"queued_bytes":0,"dropped_content_records":0,"write_failures":0,"completed_records":0,"worker_alive":true}
}
```

`runtime` is absent when rotation is unmanaged. The eight field names above are the exact persisted enum values. `PUT /admin/api/log-settings` accepts `{revision,settings}` and returns the updated wrapper; stale revisions return 409. Settings persist across restart. All settings and cleanup mutations use the existing admin session/CSRF checks—there is no new authentication mechanism.

Ranges/semantics:

* `enabled:false` stops creation of new request-log records. Existing records remain queryable and cleanable.
* `capture_fields:[]` selects no content. Headers are content too: they are **not** saved at 0 hours, even if selected.
* `body_retention_hours`: 0–87600. **0 disables new content capture, not unlimited retention.** Positive values set an expiry from the request's creation time.
* Settings are snapshotted for new requests. Requests already in progress retain their snapshot. Shortening retention or setting 0 does not retroactively change existing expiry timestamps or in-flight snapshots; use explicit field cleanup to remove existing content immediately.
* `row_retention.mode`: `manual`, `days`, or `forever`. Only `days` automatically removes log rows and their content older than `days` (1–36500); manual/forever do not automatically delete summaries. The numeric `days` value may remain stored when another mode is selected.
* `limits.per_request_bytes`: 1–16777216, shared across the request and all attempts/stages, not per field. Capture limits are not inference-body limits.
* `limits.total_body_bytes`: 0–1 TiB. 0 means zero body-storage capacity, not unlimited. Lowering this limit evicts oldest available content; summaries remain and explicitly say `evicted`.
* `runtime.file_max_bytes`: 1024–1 GiB; `retained_files`: 1–32, including the current file. Runtime rotation affects only the supervisor-owned runtime files.

Expiration/quota cleanup runs in the background about every 30 seconds. Already-expired fields are projected as unavailable immediately in details, even before physical GC. Reading expired content also cleans it and returns 410. Automatic row retention targets **only the new request-log tables/files**; it never deletes old `request_stats`, channel statistics, billing data, S3 facts or request spool files.

## Capture/writer contract

`RequestLogs` is cloneable. `settings()` is a synchronous snapshot and `new_id()` returns an internal unique id unrelated to the caller's `request_id`.

1. `begin(LogRecord) -> bool` nonblockingly reserves an active slot and queues the one-time metadata INSERT. Start messages contain no captured content. If it returns false, capture must not retain raw data.
2. `enqueue(LogRecord) -> bool` submits the final record and original captured byte vectors. Its SQL operation is **UPDATE-only**; a deleted row cannot be recreated by a late terminal callback.
3. `fail(id,reason) -> bool` terminates an existing active log on early capture failure without inserting a new row.

Normal writer queue capacity is 64 with a separate 32 MiB queued-body byte cap. There are at most 1024 active slots. Queue saturation discards content, reports `queue_full`, and routes terminal metadata through an independently bounded queue sized for the active slots. A terminal message that overtakes its queued Begin is held by id until Begin is processed; it is not applied to zero rows and forgotten. Each id accepts only one terminal submission. The capture memory reservation travels with the final record until writing/drop, including the metadata-only fallback.

SQLite/file work occurs on the dedicated `miora-request-logs` thread. Capture/redaction and production observation are implemented in `capture.rs`; they must preserve normal request/response behavior. The capture layer enforces a bounded aggregate memory budget and records omitted attempt detail rather than retaining unbounded retry metadata. `attempt_count` is the actual total; `omitted_attempts` explicitly describes skipped detail. The writer exposes queue/drop/write-failure counters instead of reporting failed capture as complete.

`LogRecord` contains id, caller request id, method, endpoint **without query**, epoch `at_ms`, channel, requested/sent/response model names, status, duration/first-byte/first-text timings, optional usage, attempt metadata, cancellation/stream/incomplete/error-stage flags, independent upstream/relay completion, expiry and byte-bearing fields. Unknown models, timing, usage or completion facts stay null; they are not guessed.

Upstream fields are stored separately per attempt in `attempt_blobs: BTreeMap<u32,...>`; retries never overwrite earlier attempt content. Use the actual returned attempt index when retrieving a field, not its position in an array.

## Content metadata and representations

Each saved field has:

```json
{
  "field":"upstream_response_body",
  "attempt":2,
  "observed_bytes":20000,
  "saved_bytes":4096,
  "capture_state":"truncated",
  "complete":false,
  "availability":"available",
  "available":true,
  "reason":"per_request_limit",
  "availability_reason":null,
  "content_type":"text/event-stream",
  "representation":"http_entity_before_protocol_conversion",
  "expires_at_ms":1890000000000,
  "redacted":true
}
```

`capture_state`, `reason`, and `complete` describe capture completeness. `availability` and `availability_reason` independently describe whether the saved bytes still exist and may be read. Expiration/clear/eviction does not pretend that initial capture was complete or incomplete; it preserves capture history while changing availability. `saved_bytes` is the byte count actually persisted at capture time and remains historical after cleanup; it is not current allocated disk usage. Cleanup reports bytes actually removed. Redaction/encoding can change byte length, so saved length need not equal observed entity length.

`complete:true` requires the entire supported entity to have been captured safely, including EOF and no truncation/discard. A retained SSE prefix is not complete just because it ends at an event boundary. `upstream_complete` and `relay_complete` are separate observations; relay EOF means the HTTP stack consumed the offered entity, **not proof the remote client received every byte**.

Representations identify the stage:

* request body after request decompression;
* upstream HTTP entity before protocol conversion;
* final relay entity offered to the HTTP stack;
* parsed HTTP header map, not original socket header formatting.

JSON, text and SSE may be retained. Binary/multipart/unsupported encoded entities are marked unsupported instead of fabricated text captures. Incomplete JSON that cannot be safely redacted is not stored as a misleading preview. The capture boundary redacts known request/provider/plugin credentials; the store additionally masks common credential keys in JSON/header maps. Raw authentication errors/URLs are not returned by the storage API. Files can still contain intentionally captured prompts and outputs, so restrict OS access and enable only the stages/retention you need.

## Query, preview and download APIs

All routes require the administrator account session. Existing `/admin/api/logs` remains administrator audit history.

| Route | Method | Result |
|---|---|---|
| `/admin/api/request-logs` | GET | `{rows,total,page,page_size}` |
| `/admin/api/request-logs/{id}` | GET | `{record,fields,active}` |
| `/admin/api/request-logs/{id}/content?field=FIELD&attempt=N&limit=65536` | GET | Field metadata plus `{content,encoding:"utf-8",preview_truncated}` |
| `/admin/api/request-logs/{id}/download?field=FIELD&attempt=N` | GET | Actual persisted bytes as a binary attachment |
| `/admin/api/request-logs/cleanup` | POST | Two-phase cleanup described below |

List filters are exact `request_id`, `channel`, `model` (matches requested/sent/response name), HTTP integer `status`, and epoch-millisecond `from_ms` inclusive / `to_ms` exclusive. Pagination defaults to page 1, 50 rows, maximum 200 rows. No undocumented success/error/range or fuzzy-search filter is implied.

Omit `attempt` for request-level fields. For upstream stages pass the field's returned attempt number, including 0 when that is the actual index. Preview defaults/maxes at 65536 bytes and sets `preview_truncated` independently of original capture truncation. An empty captured entity is different from a field that was never captured.

Missing records/fields return 404; unavailable, expired, cleared, evicted or missing content returns 410, not a fake empty download. Downloads use `application/octet-stream`, `Content-Disposition: attachment`, `nosniff`, and `no-store`, regardless of captured HTML/SVG MIME type. Partial captures have a `-partial` filename and `X-Log-Content-Complete:false`. UI previews must use text rendering, never execute captured markup. Download means the retained sanitized bytes, not a promise that a truncated capture contains the original whole response.

## Frozen two-phase cleanup

Preview:

```json
{
  "dry_run":true,
  "action":"clear_fields",
  "fields":["request_body","upstream_response_body"],
  "filters":{"channel":"example","to_ms":1890000000000,"ids":["optional-internal-id"]}
}
```

`filters` has the same names as list filters, plus optional `ids` (up to 1000 explicitly selected ids). `clear_fields` requires selected fields and applies them across all attempts; `delete_rows` removes the matching new log summaries and their body files. It does not delete original statistics.

The result includes `{dry_run:true,confirmation_token,expires_at_ms,matched_rows,active_excluded,estimated_bytes,action,fields,filters}`. `matched_rows` is the eligible inactive target count; active rows are separately reported and excluded. The target **ids are frozen in SQLite**, not just the filter expression. This dry run writes only its temporary plan/target set, not a deletion or content clear.

Confirm within five minutes:

```json
{"dry_run":false,"confirmation_token":"token-from-preview"}
```

The stored action/fields/targets are used; newly created rows are not reevaluated into the cleanup. Result: `{dry_run:false,action,affected_rows,cleared_fields,freed_bytes,skipped_rows}`. Missing/already-used/expired tokens return 409. Records that disappeared, became active or could not be cleared are skipped/reported. Content cleanup retains the summary and field capture history. Terminal callbacks never UPSERT deleted records.

## Verification scope and limitations

Isolated module tests cover 0-hour no-content behavior; expiry projected before GC; quota eviction with preserved rows; frozen dry-run/confirmation and active exclusions; late-final non-resurrection; keeping an independent statistics DB unchanged; secret masking and real download availability; day retention; settings CAS/restart persistence; and forced queue saturation with terminal-before-Begin ordering; and an explicitly unavailable/non-capturing stub after log-store initialization failure. They use temporary directories and local SQLite only. The parent runs a separate local mock inference E2E; no real supplier is needed.

The store is per-instance, not a distributed log service. Metadata can grow indefinitely in manual/forever mode; the body quota is not a metadata-index quota. File-system failures are reported and cannot guarantee capture durability. Runtime log rotation is effective only under the declared owning supervisor. No change here deletes or rewrites the old statistics database.

## Local verification commands

Run only the relevant phase when iterating on logging (Windows GNU example):

```powershell
cargo test --locked --offline --target x86_64-pc-windows-gnu request_logs --lib -- --test-threads=1
python tests/http/verify_request_logs.py target/x86_64-pc-windows-gnu/debug/uni-api-front.exe
python tests/http/verify_request_log_routes.py target/x86_64-pc-windows-gnu/debug/uni-api-front.exe
python tests/ops/test_local_preview_logging.py -v
python tests/ops/verify_local_preview_supervisor.py target/x86_64-pc-windows-gnu/debug/uni-api-front.exe
```

The route fixture includes native Responses exact-byte relay, sanitized upstream errors, moderation refusal without replacing the client's requested model/stream flag, and direct moderation with a trailing slash. Auxiliary moderation attempts are labelled separately; request-level sent-model summaries come from actual prepared inference attempts, never a guessed statistics model.

The supervisor fixture starts a temporary gateway, authenticates with a generated password hash, updates managed runtime settings through the real API, and observes the live supervisor apply the saved limit. Its files, credentials and SQLite database are separate from the local preview. Neither fixture calls paid suppliers or deletes the existing preview statistics.

These checks do not establish WAN/TLS behavior, cancellation before response headers, real redirects/trailers, slow-consumer resource bounds, or a complete external tool-execution round trip. They do not constitute a full historical regression suite or an exhaustive dependency-license audit.
