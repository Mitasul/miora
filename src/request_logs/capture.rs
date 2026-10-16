//! Observation-only four-stage logging. No disk I/O, awaits or route decisions in
//! the body poll path. The final record owns its buffer reservation until the
//! independent bounded writer finishes (or discards) it.
use super::{
    CaptureField, CapturedBlob, LogRecord, RequestLogs, SettingsSnapshot, MAX_CAPTURE_BYTES_GLOBAL,
};
use axum::body::Body;
use axum::http::{HeaderMap, Method, Response};
use bytes::Bytes;
use http_body::{Body as HttpBody, Frame, SizeHint};
use reqwest::ResponseBuilderExt;
use serde::ser::{SerializeSeq, Serializer};
use serde::Serialize;
use serde_json::{json, Value};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::task::{Context, Poll};
use std::time::Instant;

#[path = "redaction.rs"]
mod redaction;
#[cfg(test)]
#[path = "capture_tests.rs"]
mod tests;

static CAPTURE_BYTES: AtomicUsize = AtomicUsize::new(0);
const METADATA_RESERVATION: usize = 1024 * 1024;
const MAX_ATTEMPT_DETAILS: usize = 128;
const MAX_LABEL_BYTES: usize = 512;
fn label(value: &str) -> String {
    if value.len() <= MAX_LABEL_BYTES {
        return value.to_owned();
    }
    let end = value
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index <= MAX_LABEL_BYTES)
        .last()
        .unwrap_or(0);
    format!("{}…", &value[..end])
}
fn mime(value: &str) -> Option<String> {
    let value = value
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    // Only classification metadata survives when header capture is off/0h.
    // Arbitrary MIME parameters or custom subtype strings can carry credentials.
    match value.as_str() {
        "application/json"
        | "text/event-stream"
        | "text/plain"
        | "text/html"
        | "text/xml"
        | "application/xml"
        | "image/svg+xml"
        | "application/javascript"
        | "application/octet-stream"
        | "multipart/form-data"
        | "application/x-www-form-urlencoded" => Some(value),
        _ if value.ends_with("+json") => Some("application/json".into()),
        _ if value.starts_with("text/") => Some("text/plain".into()),
        _ if value.is_empty() => None,
        _ => Some("application/octet-stream".into()),
    }
}
/// Covers retained raw capture plus redacted output/scratch buffer capacity.
/// This is a bounded logging-buffer reservation, not a process RSS accounting API.
#[derive(Debug)]
pub(crate) struct CaptureReservation {
    bytes: usize,
    keep: usize,
}
impl CaptureReservation {
    fn acquire(bytes: usize) -> Option<Self> {
        CAPTURE_BYTES
            .fetch_update(Ordering::AcqRel, Ordering::Relaxed, |current| {
                current
                    .checked_add(bytes)
                    .filter(|next| *next <= MAX_CAPTURE_BYTES_GLOBAL)
            })
            .ok()
            .map(|_| Self { bytes, keep: 0 })
    }
    fn grow(&mut self, bytes: usize) -> bool {
        if CAPTURE_BYTES
            .fetch_update(Ordering::AcqRel, Ordering::Relaxed, |current| {
                current
                    .checked_add(bytes)
                    .filter(|next| *next <= MAX_CAPTURE_BYTES_GLOBAL)
            })
            .is_ok()
        {
            self.bytes += bytes;
            true
        } else {
            false
        }
    }
    pub(crate) fn release_raw(&mut self) {
        let released = self.bytes.saturating_sub(self.keep);
        self.bytes -= released;
        CAPTURE_BYTES.fetch_sub(released, Ordering::AcqRel);
    }
}
impl Drop for CaptureReservation {
    fn drop(&mut self) {
        CAPTURE_BYTES.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

#[derive(Clone)]
pub(crate) struct LogContext(Arc<Inner>);
impl std::fmt::Debug for LogContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogContext")
            .field("id", &self.0.id)
            .finish_non_exhaustive()
    }
}
struct Inner {
    id: String,
    logs: RequestLogs,
    settings: SettingsSnapshot,
    plugins: Arc<crate::plugins::PluginRegistry>,
    started: Instant,
    state: Mutex<Data>,
    raw_limit: usize,
    submit: bool,
    #[cfg(test)]
    count_locks: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    lock_count: AtomicUsize,
}
#[derive(Default)]
struct Data {
    record: LogRecord,
    slots: BTreeMap<(Option<u32>, CaptureField), Slot>,
    attempts: BTreeMap<u32, Attempt>,
    secrets: Vec<String>,
    secret_bytes: usize,
    retained_bytes: usize,
    // Spare capacity is real reserved capture memory; never charge only lengths.
    raw_capacity: usize,
    raw_failed: bool,
    raw_available: bool,
    total_attempts: u64,
    winning_attempt: Option<u32>,
    winning_omitted: bool,
    client_metadata_set: bool,
}
impl Data {
    fn fail_raw(&mut self, reason: &'static str) {
        self.raw_failed = true;
        self.record.capture_failure = Some(reason.into());
        for slot in self.slots.values_mut() {
            slot.raw = Vec::new();
            if !matches!(slot.state, "disabled" | "retention_zero" | "unsupported") {
                slot.state = "redaction_failed";
                slot.reason = Some(reason.into());
            }
        }
        self.retained_bytes = 0;
        self.raw_capacity = 0;
    }
    // Pressure-only: normal chunks never scan the slot collection. Reclaiming
    // spare capacity avoids truncating bytes which still fit the logical limit.
    fn reclaim_spare(&mut self) {
        if self.raw_capacity == self.retained_bytes {
            return;
        }
        for slot in self.slots.values_mut() {
            let before = slot.raw.capacity();
            if before > slot.raw.len() {
                slot.raw.shrink_to_fit();
                self.raw_capacity -= before - slot.raw.capacity();
            }
        }
    }
    fn add_secret(&mut self, secret: &str, limit: usize) {
        if secret.is_empty()
            || !self.raw_available
            || self.raw_failed
            || self.secrets.iter().any(|s| s == secret)
        {
            return;
        }
        if self.secrets.len() >= 128
            || self.secret_bytes.saturating_add(secret.len()) > limit.min(64 * 1024)
            || self
                .retained_bytes
                .saturating_add(self.secret_bytes)
                .saturating_add(secret.len())
                > limit
        {
            self.fail_raw("redaction_secret_budget_exceeded");
            return;
        }
        if self
            .raw_capacity
            .saturating_add(self.secret_bytes)
            .saturating_add(secret.len())
            > limit
        {
            self.reclaim_spare();
        }
        let owned = secret.to_owned();
        let capacity = owned.capacity();
        if self
            .raw_capacity
            .saturating_add(self.secret_bytes)
            .saturating_add(capacity)
            > limit
            || self.secret_bytes.saturating_add(capacity) > limit.min(64 * 1024)
        {
            self.fail_raw("redaction_secret_budget_exceeded");
            return;
        }
        self.secret_bytes += capacity;
        self.secrets.push(owned);
    }
    fn capacity_limit(&self, current_capacity: usize, limit: usize) -> usize {
        limit
            .saturating_sub(self.secret_bytes)
            .saturating_sub(self.raw_capacity.saturating_sub(current_capacity))
    }
    fn append_raw(&mut self, key: (Option<u32>, CaptureField), bytes: &[u8], limit: usize) {
        let available = limit.saturating_sub(self.retained_bytes.saturating_add(self.secret_bytes));
        let take = bytes.len().min(available);
        let slot = &self.slots[&key];
        if take > 0
            && slot.raw.len().saturating_add(take) > self.capacity_limit(slot.raw.capacity(), limit)
        {
            self.reclaim_spare();
        }
        let cap_limit = self.capacity_limit(self.slots[&key].raw.capacity(), limit);
        let slot = self.slots.get_mut(&key).expect("capture slot exists");
        let before_len = slot.raw.len();
        let before_capacity = slot.raw.capacity();
        if take > 0 {
            if redaction::reserve_bounded(&mut slot.raw, take, cap_limit).is_ok() {
                slot.raw.extend_from_slice(&bytes[..take]);
                slot.state = "collecting";
            } else {
                slot.state = "memory_limit";
                slot.reason = Some("capture_allocation_failed".into());
            }
        }
        if take < bytes.len() {
            slot.clipped = true;
            slot.reason = Some("request_capture_size_limit".into());
        }
        // Allocation failure must not consume phantom retained bytes.
        self.retained_bytes = self.retained_bytes - before_len + slot.raw.len();
        self.raw_capacity = self.raw_capacity - before_capacity + slot.raw.capacity();
    }
}

fn sensitive_header(name: &str, extra: &[String]) -> bool {
    redaction::sensitive(name)
        || crate::providers::parameters::header_sensitive(name, false)
        || extra.iter().any(|s| s.eq_ignore_ascii_case(name))
}
fn collect_header_secrets(
    state: &mut Data,
    headers: &HeaderMap,
    extra: &[String],
    limit: usize,
    mut masks: Option<&mut Vec<bool>>,
) {
    if !state.raw_available || state.raw_failed {
        return;
    }
    for (name, value) in headers {
        if state.raw_failed {
            break;
        }
        let masked = sensitive_header(name.as_str(), extra);
        if let Some(masks) = masks.as_mut() {
            masks.push(masked);
        }
        if masked {
            if let Ok(value) = value.to_str() {
                state.add_secret(value, limit);
                if let Some((scheme, token)) = value.split_once(' ') {
                    if scheme.eq_ignore_ascii_case("bearer") || scheme.eq_ignore_ascii_case("basic")
                    {
                        state.add_secret(token, limit);
                    }
                }
            }
        }
    }
}
#[derive(Serialize)]
struct HeaderRow<'a> {
    name: &'a str,
    value: Cow<'a, str>,
}
struct HeaderRows<'a> {
    headers: &'a HeaderMap,
    masks: &'a [bool],
    secrets: &'a [String],
}
impl Serialize for HeaderRows<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(Some(self.headers.len()))?;
        for ((name, value), masked) in self.headers.iter().zip(self.masks) {
            let value = if *masked {
                Cow::Borrowed(redaction::MASK)
            } else {
                Cow::Owned(redaction::clean_text(
                    value.to_str().map_err(serde::ser::Error::custom)?,
                    self.secrets,
                ))
            };
            sequence.serialize_element(&HeaderRow {
                name: name.as_str(),
                value,
            })?;
        }
        sequence.end()
    }
}
struct Attempt {
    id: String,
    started: Instant,
    meta: Value,
    service: Option<String>,
}
struct Slot {
    raw: Vec<u8>,
    observed: u64,
    ended: bool,
    eof: bool,
    clipped: bool,
    state: &'static str,
    reason: Option<String>,
    kind: redaction::Kind,
    content_type: Option<String>,
    representation: Option<String>,
}
impl Slot {
    fn new(settings: &SettingsSnapshot, field: CaptureField, reserved: bool) -> Self {
        let (state, reason) = if !settings.settings.capture_fields.contains(&field) {
            ("disabled", Some("field_disabled".into()))
        } else if settings.settings.body_retention_hours == 0 {
            ("retention_zero", Some("body_retention_zero".into()))
        } else if !reserved {
            (
                "memory_limit",
                Some("global_capture_budget_unavailable".into()),
            )
        } else {
            ("not_observed", None)
        };
        Self {
            raw: Vec::new(),
            observed: 0,
            ended: false,
            eof: false,
            clipped: false,
            state,
            reason,
            kind: redaction::Kind::Unsupported,
            content_type: None,
            representation: None,
        }
    }
    fn collecting(&self) -> bool {
        matches!(self.state, "not_observed" | "collecting")
            && !self.clipped
            && self.kind != redaction::Kind::Unsupported
    }
}
#[derive(Clone)]
pub(crate) struct AttemptCapture {
    context: LogContext,
    index: u32,
    recorded: bool,
    sensitive_headers: Arc<Vec<String>>,
}
#[derive(Clone)]
struct Sink {
    context: LogContext,
    key: (Option<u32>, CaptureField),
}

impl LogContext {
    pub(crate) fn start(
        logs: &RequestLogs,
        plugins: Arc<crate::plugins::PluginRegistry>,
        request_id: &str,
        method: &Method,
        path: &str,
        headers: &HeaderMap,
    ) -> Option<Self> {
        let settings = logs.settings();
        if !settings.backend_enabled || !settings.settings.enabled {
            return None;
        }
        let id = logs.new_id();
        let at_ms = super::now_ms();
        let initial = LogRecord {
            id: id.clone(),
            request_id: request_id.chars().take(512).collect(),
            at_ms,
            method: Some(method.to_string()),
            endpoint: Some(label(path)),
            ..Default::default()
        };
        if !logs.begin(initial) {
            return None;
        }
        let Some(mut reservation) = CaptureReservation::acquire(METADATA_RESERVATION) else {
            logs.fail(&id, "capture_metadata_budget_unavailable");
            return None;
        };
        reservation.keep = METADATA_RESERVATION;
        let wants_raw = settings.settings.body_retention_hours > 0
            && !settings.settings.capture_fields.is_empty();
        let raw_limit = (settings
            .settings
            .limits
            .per_request_bytes
            .min(usize::MAX as u64) as usize)
            .min((MAX_CAPTURE_BYTES_GLOBAL - METADATA_RESERVATION) / 2);
        let raw_available = wants_raw && reservation.grow(raw_limit.saturating_mul(2));
        let record = LogRecord {
            id: id.clone(),
            request_id: request_id.chars().take(512).collect(),
            at_ms,
            method: Some(method.to_string()),
            endpoint: Some(label(path)),
            body_expires_at_ms: settings.body_expires_at_ms(at_ms),
            capture_budget: Some(reservation),
            ..Default::default()
        };
        let mut data = Data {
            record,
            raw_available,
            ..Default::default()
        };
        for field in CaptureField::ALL {
            data.slots.insert(
                (None, field),
                Slot::new(&settings, field, data.raw_available),
            );
        }
        if wants_raw && !data.raw_available {
            data.record.capture_failure = Some("global_capture_budget_unavailable".into());
        }
        initialize_worker();
        let context = Self(Arc::new(Inner {
            id,
            logs: logs.clone(),
            settings,
            plugins,
            started: Instant::now(),
            state: Mutex::new(data),
            raw_limit,
            submit: true,
            #[cfg(test)]
            count_locks: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            lock_count: AtomicUsize::new(0),
        }));
        if raw_available {
            if let Some(key) = crate::routing::access::extract_api_key(headers) {
                context.add_secret(&key);
            }
        }
        context.headers((None, CaptureField::RequestHeaders), headers, &[]);
        Some(context)
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, Data> {
        #[cfg(test)]
        if self.0.count_locks.load(Ordering::Relaxed) {
            self.0.lock_count.fetch_add(1, Ordering::Relaxed);
        }
        self.0
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    fn limit(&self) -> usize {
        self.0.raw_limit
    }
    pub(crate) fn add_secret(&self, secret: &str) {
        if secret.is_empty()
            || self.0.settings.settings.body_retention_hours == 0
            || self.0.settings.settings.capture_fields.is_empty()
        {
            return;
        }
        self.lock().add_secret(secret, self.limit());
    }
    fn header_secrets(&self, headers: &HeaderMap, extra: &[String]) {
        let mut state = self.lock();
        collect_header_secrets(&mut state, headers, extra, self.limit(), None);
    }
    fn headers(&self, key: (Option<u32>, CaptureField), headers: &HeaderMap, extra: &[String]) {
        // One lock, even for multiple Bearer/Basic/declared-secret headers. A
        // disabled header field still supplies secrets when another raw field
        // is enabled. In metadata-only mode no secret classification is needed.
        let mut state = self.lock();
        let mut masks = Vec::new();
        let capture_header = state.raw_available
            && !state.raw_failed
            && self.0.settings.settings.capture_fields.contains(&key.1);
        collect_header_secrets(
            &mut state,
            headers,
            extra,
            self.limit(),
            capture_header.then_some(&mut masks),
        );
        let observed = headers
            .iter()
            .map(|(name, value)| name.as_str().len() as u64 + value.as_bytes().len() as u64)
            .sum();
        let reserved = state.raw_available && !state.raw_failed;
        let slot = state
            .slots
            .entry(key)
            .or_insert_with(|| Slot::new(&self.0.settings, key.1, reserved));
        slot.observed = observed;
        slot.ended = true;
        slot.eof = true;
        slot.kind = redaction::Kind::Json;
        slot.content_type = Some("application/json".into());
        slot.representation = Some("parsed_http_header_map_name_value_octets".into());
        if !slot.collecting() {
            return;
        }
        let previous_len = slot.raw.len();
        let available = self.limit().saturating_sub(
            state
                .retained_bytes
                .saturating_sub(previous_len)
                .saturating_add(state.secret_bytes),
        );
        if observed > available as u64 {
            let slot = state.slots.get_mut(&key).expect("capture slot exists");
            slot.clipped = true;
            slot.state = "size_limit";
            slot.reason = Some("request_capture_size_limit".into());
            return;
        }
        if headers.values().any(|value| value.to_str().is_err()) {
            let slot = state.slots.get_mut(&key).expect("capture slot exists");
            slot.state = "unsupported";
            slot.reason = Some("non_text_header_value".into());
            return;
        }
        // Borrow the secret set and serialize rows directly; avoid cloning all
        // secrets and allocating a temporary JSON object for every header.
        let encoded = redaction::encode(
            &HeaderRows {
                headers,
                masks: &masks,
                secrets: &state.secrets,
            },
            available,
        );
        match encoded {
            Ok(bytes) => {
                let current = state.slots[&key].raw.capacity();
                if bytes.capacity() > state.capacity_limit(current, self.limit()) {
                    state.reclaim_spare();
                }
                let current = state.slots[&key].raw.capacity();
                if bytes.capacity() > state.capacity_limit(current, self.limit()) {
                    let slot = state.slots.get_mut(&key).expect("capture slot exists");
                    slot.state = "memory_limit";
                    slot.reason = Some("capture_allocation_failed".into());
                    return;
                }
                let slot = state.slots.get_mut(&key).expect("capture slot exists");
                let old_len = slot.raw.len();
                let old_capacity = slot.raw.capacity();
                let len = bytes.len();
                let capacity = bytes.capacity();
                slot.raw = bytes;
                slot.state = "collecting";
                state.retained_bytes = state.retained_bytes - old_len + len;
                state.raw_capacity = state.raw_capacity - old_capacity + capacity;
            }
            Err(()) => {
                let slot = state.slots.get_mut(&key).expect("capture slot exists");
                slot.clipped = true;
                slot.state = "size_limit";
                slot.reason = Some("request_capture_size_limit".into());
            }
        }
    }
    fn body_sink(
        &self,
        key: (Option<u32>, CaptureField),
        headers: &HeaderMap,
        representation: &str,
    ) -> Sink {
        let content_type = headers
            .get("content-type")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("");
        let encoding = headers
            .get("content-encoding")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("");
        let mut state = self.lock();
        let reserved = state.raw_available && !state.raw_failed;
        let slot = state
            .slots
            .entry(key)
            .or_insert_with(|| Slot::new(&self.0.settings, key.1, reserved));
        slot.content_type = mime(content_type);
        slot.kind = redaction::kind(content_type, encoding);
        slot.representation = Some(
            if !encoding.is_empty() && !encoding.eq_ignore_ascii_case("identity") {
                "encoded_http_entity_unsupported".into()
            } else {
                representation.into()
            },
        );
        if slot.state == "not_observed" && slot.kind == redaction::Kind::Unsupported {
            slot.state = "unsupported";
            slot.reason = Some("unsupported_binary_multipart_or_encoding".into());
        }
        Sink {
            context: self.clone(),
            key,
        }
    }
    pub(crate) fn client_request(
        &self,
        mut request: axum::extract::Request,
    ) -> axum::extract::Request {
        let sink = self.body_sink(
            (None, CaptureField::RequestBody),
            request.headers(),
            "decoded_after_request_decompression",
        );
        let body = std::mem::replace(request.body_mut(), Body::empty());
        *request.body_mut() = Body::new(ObservedBody::new(body, sink));
        request
    }
    pub(crate) fn response(&self, mut response: Response<Body>) -> Response<Body> {
        {
            let mut state = self.lock();
            state.record.status = Some(response.status().as_u16());
        }
        self.headers(
            (None, CaptureField::ResponseHeaders),
            response.headers(),
            &[],
        );
        let sink = self.body_sink(
            (None, CaptureField::ResponseBody),
            response.headers(),
            "relay_entity_offered_to_http_stack_not_client_acknowledgement",
        );
        let body = std::mem::replace(response.body_mut(), Body::empty());
        *response.body_mut() = Body::new(ObservedBody::new(body, sink));
        response
    }
    pub(crate) fn client_metadata(&self, model: &str, stream: bool) {
        let mut state = self.lock();
        state.record.requested_model = Some(label(model));
        state.record.stream = stream;
        state.client_metadata_set = true;
    }
    pub(crate) fn error_stage(&self, stage: &'static str) {
        self.lock().record.error_stage = Some(stage.to_owned());
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn begin_attempt(
        &self,
        attempt_id: &str,
        channel: Option<&str>,
        requested: Option<&str>,
        sent: Option<&str>,
        stream: bool,
        provider_key: Option<&str>,
        sensitive_headers: &[String],
        plugin: Option<&str>,
    ) -> AttemptCapture {
        let mut state = self.lock();
        if let Some(key) = provider_key {
            state.add_secret(key, self.limit());
            if key.contains(',') {
                for part in key.split(',').filter(|part| !part.is_empty()).take(8) {
                    state.add_secret(part, self.limit());
                }
            }
        }
        state.total_attempts = state.total_attempts.saturating_add(1);
        state.record.attempt_count = state.total_attempts;
        if state.attempts.len() >= MAX_ATTEMPT_DETAILS {
            state.record.omitted_attempts = state.record.omitted_attempts.saturating_add(1);
            state.record.capture_failure = Some("attempt_capture_limit".into());
            return AttemptCapture {
                context: self.clone(),
                index: 0,
                recorded: false,
                sensitive_headers: Arc::new(Vec::new()),
            };
        }
        let index = state.attempts.len() as u32 + 1;
        let meta = json!({"index":index,"attempt_id":label(attempt_id),"channel":channel.map(label),"requested_model":requested.map(label),"sent_model":sent.map(label),"response_model":null,"status":null,"duration_ms":null,"first_byte_ms":null,"first_text_ms":null,"usage":null,"upstream_complete":null,"request_complete":null,"cancelled":false,"error_stage":null,"stream":stream,"endpoint":state.record.endpoint,"phase":"inference"});
        state.attempts.insert(
            index,
            Attempt {
                id: label(attempt_id),
                started: Instant::now(),
                meta,
                service: plugin.filter(|id| id.len() <= 128).map(str::to_owned),
            },
        );
        if state.record.requested_model.is_none() {
            state.record.requested_model = requested.map(label);
        }
        state.record.channel = channel.map(label);
        state.record.sent_model = sent.map(label);
        if !state.client_metadata_set {
            state.record.stream = stream;
        }
        let reserved = state.raw_available && !state.raw_failed;
        for field in [
            CaptureField::UpstreamRequestHeaders,
            CaptureField::UpstreamRequestBody,
            CaptureField::UpstreamResponseHeaders,
            CaptureField::UpstreamResponseBody,
        ] {
            state.slots.insert(
                (Some(index), field),
                Slot::new(&self.0.settings, field, reserved),
            );
        }
        let extra_size = sensitive_headers.iter().map(String::len).sum::<usize>();
        let extra = if reserved && extra_size <= 4096 && sensitive_headers.len() <= 64 {
            sensitive_headers.to_vec()
        } else {
            Vec::new()
        };
        if reserved && (extra_size > 4096 || sensitive_headers.len() > 64) {
            state.fail_raw("sensitive_header_metadata_limit");
        }
        AttemptCapture {
            context: self.clone(),
            index,
            recorded: true,
            sensitive_headers: Arc::new(extra),
        }
    }
    pub(crate) fn record_request(&self, stat: &crate::storage::database::RequestStat) {
        let mut state = self.lock();
        if !stat.model.is_empty() {
            state.record.requested_model = Some(label(&stat.model));
        }
        if !stat.upstream_model.is_empty() {
            state.record.sent_model = Some(label(&stat.upstream_model));
        }
        if !stat.provider.is_empty() {
            state.record.channel = Some(label(&stat.provider));
        }
        state.record.status.get_or_insert(stat.status);
        state.record.stream = stat.stream;
        state.record.first_text_ms = stat.first_text_ms.or(state.record.first_text_ms);
        if let Some(usage) = redaction::usage_json(&stat.fact_usage) {
            state.record.usage = Some(usage);
        }
        if stat.response_completed == Some(false) {
            state.record.incomplete = true;
        }
        if stat.status == 499 || stat.failure_reason.as_deref() == Some("downstream_disconnected") {
            state.record.cancelled = true;
        }
        if let Some(reason) = &stat.failure_reason {
            state.record.error_stage = Some(failure_stage(reason).into());
        }
    }
    pub(crate) fn record_channel(&self, stat: &crate::storage::database::ChannelStat) {
        let attempt_id = label(&stat.attempt_id);
        let mut state = self.lock();
        state.add_secret(&stat.provider_api_key, self.limit());
        let found = state
            .attempts
            .iter()
            .rev()
            .find(|(_, a)| a.id == attempt_id)
            .map(|(index, _)| *index);
        if let Some(index) = found {
            let attempt = state.attempts.get_mut(&index).unwrap();
            attempt.meta["semantic_success"] = json!(stat.success);
            attempt.meta["first_text_ms"] = json!(stat.first_text_ms);
            attempt.meta["duration_ms"] = json!(stat
                .duration_ms
                .or(Some(attempt.started.elapsed().as_secs_f64() * 1000.0)));
            attempt.meta["semantic_complete"] = json!(stat.response_completed);
            if let Some(reason) = &stat.failure_reason {
                attempt.meta["error_stage"] = json!(failure_stage(reason));
            }
            if stat.success {
                state.winning_attempt = Some(index);
            }
        } else if stat.success {
            state.winning_omitted = true;
        }
    }
}
fn failure_stage(reason: &str) -> &'static str {
    if reason.contains("downstream") {
        "relay"
    } else if reason.contains("transport") || reason.contains("timeout") {
        "upstream_transport"
    } else if reason.contains("protocol") || reason.contains("precommit") {
        "upstream_protocol"
    } else {
        "upstream"
    }
}

impl Sink {
    fn feed(&self, bytes: &[u8]) {
        let mut state = self.context.lock();
        let collect = if let Some(slot) = state.slots.get_mut(&self.key) {
            slot.observed = slot.observed.saturating_add(bytes.len() as u64);
            slot.collecting()
        } else {
            false
        };
        if collect && state.raw_available && !state.raw_failed {
            state.append_raw(self.key, bytes, self.context.limit());
        }
        if !bytes.is_empty() {
            if let Some(index) = self.key.0 {
                if self.key.1 == CaptureField::UpstreamResponseBody {
                    if let Some(attempt) = state.attempts.get_mut(&index) {
                        if attempt.meta["first_byte_ms"].is_null() {
                            attempt.meta["first_byte_ms"] =
                                json!(attempt.started.elapsed().as_secs_f64() * 1000.0);
                        }
                    }
                    if state.record.first_byte_ms.is_none() {
                        state.record.first_byte_ms =
                            Some(self.context.0.started.elapsed().as_secs_f64() * 1000.0);
                    }
                }
            }
        }
    }
    fn finish(&self, eof: bool, reason: Option<&str>) {
        let mut state = self.context.lock();
        let Some(slot) = state.slots.get_mut(&self.key) else {
            return;
        };
        if slot.ended {
            return;
        }
        slot.ended = true;
        slot.eof = eof;
        if !eof && slot.reason.is_none() {
            slot.reason = reason.map(str::to_owned);
        }
        match self.key {
            (None, CaptureField::ResponseBody) => {
                state.record.relay_complete = Some(eof);
                state.record.duration_ms =
                    Some(self.context.0.started.elapsed().as_secs_f64() * 1000.0);
                if !eof {
                    state.record.incomplete = true;
                    state.record.cancelled = reason == Some("body_dropped_before_eof");
                    state.record.error_stage = Some("relay".into());
                }
            }
            (Some(index), field) => {
                if let Some(attempt) = state.attempts.get_mut(&index) {
                    if field == CaptureField::UpstreamResponseBody {
                        attempt.meta["upstream_complete"] = json!(eof);
                        attempt.meta["duration_ms"] =
                            json!(attempt.started.elapsed().as_secs_f64() * 1000.0);
                        if !eof {
                            attempt.meta["cancelled"] =
                                json!(reason == Some("body_dropped_before_eof"));
                            attempt.meta["error_stage"] = json!("upstream_body");
                        }
                    } else if field == CaptureField::UpstreamRequestBody {
                        attempt.meta["request_complete"] = json!(eof);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Generic frame observer retains size hints, trailers and the exact error type.
struct ObservedBody<B: HttpBody<Data = Bytes>> {
    inner: Pin<Box<B>>,
    sink: Option<Sink>,
}
impl<B: HttpBody<Data = Bytes>> ObservedBody<B> {
    fn new(inner: B, sink: Sink) -> Self {
        if inner.is_end_stream() {
            sink.finish(true, None);
        }
        Self {
            inner: Box::pin(inner),
            sink: Some(sink),
        }
    }
}
impl<B: HttpBody<Data = Bytes>> HttpBody for ObservedBody<B> {
    type Data = Bytes;
    type Error = B::Error;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, B::Error>>> {
        let output = self.inner.as_mut().poll_frame(cx);
        if let Some(sink) = &self.sink {
            match &output {
                Poll::Ready(Some(Ok(frame))) => {
                    if let Some(bytes) = frame.data_ref() {
                        sink.feed(bytes);
                    }
                    if self.inner.is_end_stream() {
                        sink.finish(true, None);
                    }
                }
                Poll::Ready(None) => sink.finish(true, None),
                Poll::Ready(Some(Err(_))) => sink.finish(false, Some("body_read_error")),
                Poll::Pending => {}
            }
        }
        output
    }
    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }
    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}
impl<B: HttpBody<Data = Bytes>> Drop for ObservedBody<B> {
    fn drop(&mut self) {
        if let Some(sink) = self.sink.take() {
            sink.finish(false, Some("body_dropped_before_eof"));
        }
    }
}

impl AttemptCapture {
    pub(crate) fn tag_endpoint(&self, endpoint: &str) {
        if !self.recorded {
            return;
        }
        let mut state = self.context.lock();
        let auxiliary = endpoint.trim_end_matches('/') == "/v1/moderations"
            && state
                .record
                .endpoint
                .as_deref()
                .is_some_and(|path| path.trim_end_matches('/') != "/v1/moderations");
        if let Some(attempt) = state.attempts.get_mut(&self.index) {
            attempt.meta["endpoint"] = json!(label(endpoint));
            attempt.meta["phase"] = json!(if auxiliary { "moderation" } else { "inference" });
        }
        if auxiliary {
            // The preflight is visible as its own attempt, never impersonates
            // the user's requested inference model in the main summary.
            state.record.sent_model = None;
            state.record.channel = None;
        }
    }
    fn request(&self, request: &mut reqwest::Request) {
        if !self.recorded {
            self.context.header_secrets(request.headers(), &[]);
            return;
        }
        self.context.headers(
            (Some(self.index), CaptureField::UpstreamRequestHeaders),
            request.headers(),
            &self.sensitive_headers,
        );
        let sink = self.context.body_sink(
            (Some(self.index), CaptureField::UpstreamRequestBody),
            request.headers(),
            "constructed_http_entity_before_transport",
        );
        if let Some(bytes) = request.body().and_then(reqwest::Body::as_bytes) {
            // Do not turn a reusable JSON/byte body into a one-shot stream: the
            // HTTP client's redirects/replay and connection retries stay intact.
            sink.feed(bytes);
            sink.finish(true, None);
        } else if let Some(body) = request.body_mut().take() {
            *request.body_mut() = Some(reqwest::Body::wrap(ObservedBody::new(body, sink)));
        } else {
            sink.finish(true, None);
        }
    }
    fn response(&self, response: reqwest::Response) -> reqwest::Response {
        if !self.recorded {
            self.context.header_secrets(response.headers(), &[]);
            return response;
        }
        {
            let mut state = self.context.lock();
            if let Some(attempt) = state.attempts.get_mut(&self.index) {
                attempt.meta["status"] = json!(response.status().as_u16());
            }
        }
        self.context.headers(
            (Some(self.index), CaptureField::UpstreamResponseHeaders),
            response.headers(),
            &[],
        );
        let sink = self.context.body_sink(
            (Some(self.index), CaptureField::UpstreamResponseBody),
            response.headers(),
            "http_entity_before_protocol_conversion",
        );
        let url = response.url().clone();
        let response: Response<reqwest::Body> = response.into();
        let (mut parts, body) = response.into_parts();
        // reqwest's public inverse conversion does not put its URL back in
        // extensions; restore it using the public ResponseBuilderExt marker.
        let marker = Response::builder()
            .url(url)
            .body(())
            .expect("URL-only response builder");
        parts.extensions.extend(marker.into_parts().0.extensions);
        reqwest::Response::from(Response::from_parts(
            parts,
            reqwest::Body::wrap(ObservedBody::new(body, sink)),
        ))
    }
    pub(crate) fn error(&self, stage: &'static str) {
        let mut state = self.context.lock();
        if let Some(attempt) = state.attempts.get_mut(&self.index) {
            attempt.meta["error_stage"] = json!(stage);
            attempt.meta["duration_ms"] = json!(attempt.started.elapsed().as_secs_f64() * 1000.0);
        }
    }
}
/// Captures the final built request after OAuth/headers/body construction. With
/// logging off this calls the original builder.send path without wrapping.
pub(crate) async fn send(
    client: &reqwest::Client,
    builder: reqwest::RequestBuilder,
    capture: Option<AttemptCapture>,
) -> Result<reqwest::Response, reqwest::Error> {
    let Some(capture) = capture else {
        return builder.send().await;
    };
    let mut request = match builder.build() {
        Ok(request) => request,
        Err(error) => {
            capture.error("upstream_request_build");
            return Err(error);
        }
    };
    capture.request(&mut request);
    match client.execute(request).await {
        Ok(response) => Ok(capture.response(response)),
        Err(error) => {
            capture.error("upstream_transport");
            Err(error)
        }
    }
}

struct FinalizeJob {
    state: Data,
    logs: RequestLogs,
    plugins: Arc<crate::plugins::PluginRegistry>,
    started: Instant,
    ended: Instant,
    raw_limit: usize,
}
static FINALIZER: OnceLock<Option<mpsc::SyncSender<FinalizeJob>>> = OnceLock::new();
pub(crate) fn initialize_worker() {
    let _ = finalizer();
}
fn finalizer() -> Option<&'static mpsc::SyncSender<FinalizeJob>> {
    FINALIZER
        .get_or_init(|| {
            let (sender, receiver) = mpsc::sync_channel::<FinalizeJob>(64);
            std::thread::Builder::new()
                .name("miora-log-redactor".into())
                .spawn(move || {
                    while let Ok(job) = receiver.recv() {
                        let logs = job.logs.clone();
                        let record = job.finalize(true, None);
                        logs.enqueue(record);
                    }
                })
                .ok()
                .map(|_| sender)
        })
        .as_ref()
}
impl FinalizeJob {
    fn finalize(self, redact: bool, failure: Option<&str>) -> LogRecord {
        let mut data = self.state;
        if let Some(reason) = failure {
            data.record.capture_failure = Some(reason.into());
        }
        let mut saved = 0usize;
        let limit = self.raw_limit;
        for ((attempt_index, field), slot) in data.slots {
            let mut blob = CapturedBlob {
                observed_bytes: slot.observed,
                content_type: slot.content_type,
                representation: slot.representation,
                ..Default::default()
            };
            if !redact && matches!(slot.state, "collecting" | "not_observed") {
                blob.capture_state = "queue_full".into();
                blob.reason = failure.map(str::to_owned);
            } else if matches!(slot.state, "collecting") {
                let plugin = attempt_index
                    .and_then(|index| data.attempts.get(&index))
                    .and_then(|attempt| attempt.service.as_deref())
                    .map(|id| (self.plugins.as_ref(), id));
                let result = redaction::redact(
                    slot.kind,
                    &slot.raw,
                    slot.eof,
                    slot.clipped,
                    limit.saturating_sub(saved),
                    &data.secrets,
                    plugin,
                );
                saved = saved.saturating_add(result.data.len());
                blob.data = result.data;
                blob.capture_state = result.state.into();
                blob.complete = result.complete;
                blob.reason = result.reason.map(str::to_owned).or(slot.reason);
                if field == CaptureField::UpstreamResponseBody {
                    if let Some(index) = attempt_index {
                        if let Some(attempt) = data.attempts.get_mut(&index) {
                            attempt.meta["response_model"] = json!(result.observed.model);
                            if let Some(usage) = redaction::usage_json(&result.observed.usage) {
                                attempt.meta["usage"] = usage;
                            }
                            if let Some(terminal) = result.observed.terminal {
                                attempt.meta["observed_terminal"] = json!(terminal);
                            }
                        }
                    }
                } else if field == CaptureField::RequestBody
                    && data.record.requested_model.is_none()
                {
                    data.record.requested_model = result.observed.model;
                }
            } else {
                blob.capture_state = slot.state.into();
                blob.reason = slot.reason;
                if blob.capture_state == "not_observed" {
                    if slot.eof && slot.ended && slot.observed == 0 {
                        blob.capture_state = "empty".into();
                        blob.complete = true;
                        blob.reason = Some("empty_entity".into());
                    } else {
                        blob.reason = Some("stage_not_observed".into());
                    }
                }
            }
            if let Some(index) = attempt_index {
                data.record
                    .attempt_blobs
                    .entry(index)
                    .or_default()
                    .insert(field, blob);
            } else {
                if matches!(
                    field,
                    CaptureField::UpstreamRequestHeaders
                        | CaptureField::UpstreamRequestBody
                        | CaptureField::UpstreamResponseHeaders
                        | CaptureField::UpstreamResponseBody
                ) && !data.attempts.is_empty()
                {
                    blob.capture_state = "per_attempt".into();
                    blob.reason = Some("select_attempt_for_upstream_capture".into());
                }
                data.record.blobs.insert(field, blob);
            }
        }
        let selected = if data.winning_omitted {
            None
        } else {
            data.winning_attempt.or_else(|| {
                (data.record.omitted_attempts == 0)
                    .then(|| {
                        data.attempts
                            .iter()
                            .rev()
                            .find(|(_, attempt)| {
                                attempt.meta["phase"].as_str() != Some("moderation")
                            })
                            .map(|(index, _)| *index)
                    })
                    .flatten()
            })
        };
        if let Some(attempt) = selected.and_then(|index| data.attempts.get(&index)) {
            data.record.sent_model = attempt.meta["sent_model"].as_str().map(str::to_owned);
            data.record.channel = attempt.meta["channel"].as_str().map(str::to_owned);
            data.record.upstream_complete = attempt.meta["upstream_complete"].as_bool();
            data.record.response_model = attempt.meta["response_model"].as_str().map(str::to_owned);
            if data.record.usage.is_none() && !attempt.meta["usage"].is_null() {
                data.record.usage = Some(attempt.meta["usage"].clone());
            }
        }
        if selected.is_none() && data.total_attempts > 0 {
            data.record.sent_model = None;
            data.record.channel = None;
        }
        data.record.attempts = data
            .attempts
            .into_values()
            .map(|mut attempt| {
                if attempt.meta["duration_ms"].is_null() {
                    attempt.meta["duration_ms"] = json!(
                        self.ended
                            .saturating_duration_since(attempt.started)
                            .as_secs_f64()
                            * 1000.0
                    );
                }
                if attempt.meta["status"].is_null() && attempt.meta["error_stage"].is_null() {
                    attempt.meta["error_stage"] = json!("upstream_headers_not_observed");
                }
                attempt.meta
            })
            .collect();
        data.record.duration_ms.get_or_insert(
            self.ended
                .saturating_duration_since(self.started)
                .as_secs_f64()
                * 1000.0,
        );
        if data.record.relay_complete != Some(true) {
            data.record.incomplete = true;
        }
        if !redact {
            if let Some(budget) = data.record.capture_budget.as_mut() {
                budget.release_raw();
            }
        }
        data.record
    }
}
impl Drop for Inner {
    fn drop(&mut self) {
        if !self.submit {
            return;
        }
        let state = std::mem::take(
            self.state
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        let job = FinalizeJob {
            state,
            logs: self.logs.clone(),
            plugins: self.plugins.clone(),
            started: self.started,
            ended: Instant::now(),
            raw_limit: self.raw_limit,
        };
        // No parsing/redaction or disk waits while an upstream Body/socket is
        // being dropped. A fixed worker and queue own all heavyweight work.
        if let Some(sender) = finalizer() {
            match sender.try_send(job) {
                Ok(()) => (),
                Err(error) => {
                    let (job, reason) = match error {
                        mpsc::TrySendError::Full(job) => (job, "capture_queue_full"),
                        mpsc::TrySendError::Disconnected(job) => {
                            (job, "capture_worker_unavailable")
                        }
                    };
                    let logs = job.logs.clone();
                    logs.enqueue(job.finalize(false, Some(reason)));
                }
            }
        } else {
            let logs = job.logs.clone();
            logs.enqueue(job.finalize(false, Some("capture_worker_unavailable")));
        }
    }
}
