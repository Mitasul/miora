#[test]
fn direct_moderation_with_trailing_slash_remains_an_inference_attempt() {
    let f = fixture(vec![], 0, 4096);
    f.context.lock().record.endpoint = Some("/v1/moderations/".into());
    f.context.client_metadata("safety-public", false);
    let attempt = f.context.begin_attempt(
        "req-r1",
        Some("safety-channel"),
        Some("safety-public"),
        Some("safety-wire"),
        false,
        None,
        &[],
        None,
    );
    attempt.tag_endpoint("/v1/moderations");
    let record = finish(&f.context);
    assert_eq!(record.attempts[0]["phase"], "inference");
    assert_eq!(record.channel.as_deref(), Some("safety-channel"));
    assert_eq!(record.sent_model.as_deref(), Some("safety-wire"));
}
use super::*;
use crate::plugins::registry::{Registration, TrustPolicy};
use crate::request_logs::{LogSettings, SettingsSnapshot};
use std::sync::MutexGuard;

static TEST_BUDGET_LOCK: Mutex<()> = Mutex::new(());
struct Fixture {
    context: LogContext,
    _guard: MutexGuard<'static, ()>,
}
fn fixture(fields: Vec<CaptureField>, hours: u32, limit: usize) -> Fixture {
    let guard = TEST_BUDGET_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let logs = RequestLogs::open(std::path::PathBuf::new(), false, false).unwrap();
    let mut settings = LogSettings {
        capture_fields: fields,
        body_retention_hours: hours,
        ..Default::default()
    };
    settings.limits.per_request_bytes = limit as u64;
    let settings = SettingsSnapshot {
        revision: 0,
        settings,
        backend_enabled: true,
        runtime_rotation_managed: false,
    };
    let wants_raw = hours > 0 && !settings.settings.capture_fields.is_empty();
    let mut budget = CaptureReservation::acquire(METADATA_RESERVATION).unwrap();
    budget.keep = METADATA_RESERVATION;
    if wants_raw {
        assert!(budget.grow(limit * 2));
    }
    let record = LogRecord {
        id: "capture-test".into(),
        request_id: "external".into(),
        at_ms: super::super::now_ms(),
        body_expires_at_ms: settings.body_expires_at_ms(super::super::now_ms()),
        capture_budget: Some(budget),
        ..Default::default()
    };
    let mut state = Data {
        record,
        raw_available: wants_raw,
        ..Default::default()
    };
    for field in CaptureField::ALL {
        state
            .slots
            .insert((None, field), Slot::new(&settings, field, wants_raw));
    }
    let plugins = Arc::new(
        crate::plugins::PluginRegistry::open(
            None,
            TrustPolicy {
                allowed_origins: vec![],
                allow_loopback_http: true,
            },
        )
        .unwrap(),
    );
    let context = LogContext(Arc::new(Inner {
        id: "capture-test".into(),
        logs,
        settings,
        plugins,
        started: Instant::now(),
        state: Mutex::new(state),
        raw_limit: limit,
        submit: false,
        count_locks: std::sync::atomic::AtomicBool::new(false),
        lock_count: AtomicUsize::new(0),
    }));
    Fixture {
        context,
        _guard: guard,
    }
}
fn finish(context: &LogContext) -> LogRecord {
    let data = std::mem::take(&mut *context.lock());
    FinalizeJob {
        state: data,
        logs: context.0.logs.clone(),
        plugins: context.0.plugins.clone(),
        started: context.0.started,
        ended: Instant::now(),
        raw_limit: context.0.raw_limit,
    }
    .finalize(true, None)
}
fn headers(kind: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("content-type", kind.parse().unwrap());
    headers
}

#[test]
fn known_tokens_and_sensitive_typed_keys_are_redacted() {
    let input=br#"{"authorization":"new-credential","password":true,"private_key":{"nested":1},"message":"echo caller-token and upstream-token"}"#;
    let result = redaction::redact(
        redaction::Kind::Json,
        input,
        true,
        false,
        4096,
        &["caller-token".into(), "upstream-token".into()],
        None,
    );
    assert!(result.complete);
    let text = String::from_utf8(result.data).unwrap();
    for secret in ["new-credential", "caller-token", "upstream-token"] {
        assert!(!text.contains(secret));
    }
    let value: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["password"], redaction::MASK);
    assert_eq!(value["private_key"], redaction::MASK);
}
#[test]
fn named_header_rows_are_redacted_in_sse_without_known_token() {
    let raw=b"data: {\"headers\":[{\"name\":\"Authorization\",\"value\":\"Bearer NEW_SECRET\",\"secret\":false},{\"name\":\"X-Opaque\",\"value\":17,\"secret\":true}]}\n\n";
    let result = redaction::redact(redaction::Kind::Sse, raw, true, false, 4096, &[], None);
    let text = String::from_utf8(result.data).unwrap();
    assert!(!text.contains("NEW_SECRET"));
    assert!(text.contains("[redacted]"));
    assert!(text.contains("\"secret\":false"));
    assert!(!text.contains("\"value\":17"));
}
#[test]
fn malformed_json_under_text_mime_is_not_saved_unredacted() {
    let result = redaction::redact(
        redaction::Kind::Text,
        br#"{"access_token":"NEW_SECRET""#,
        true,
        false,
        4096,
        &[],
        None,
    );
    assert!(result.data.is_empty());
    assert_eq!(result.state, "redaction_failed");
}
#[test]
fn redaction_expansion_is_limited_and_masks_are_not_reprocessed() {
    let result = redaction::redact(
        redaction::Kind::Text,
        &vec![b'a'; 128 * 1024],
        true,
        false,
        1024,
        &["a".into(), "e".into(), "d".into()],
        None,
    );
    assert!(result.data.is_empty());
    assert_eq!(result.state, "size_limit");
    assert_eq!(
        redaction::clean_text("a", &["a".into(), "redacted".into()]),
        "[redacted]"
    );
    let repeated = "abcdefghijklmnopqrstuvwx".repeat(16384);
    let result = redaction::redact(
        redaction::Kind::Text,
        repeated.as_bytes(),
        true,
        false,
        repeated.len(),
        &[
            "Bearer abcdefghijklmnopqrstuvwx".into(),
            "abcdefghijklmnopqrstuvwx".into(),
        ],
        None,
    );
    assert!(result.complete);
    assert!(!String::from_utf8_lossy(&result.data).contains("abcdefghijklmnopqrstuvwx"));
}
#[test]
fn sse_saves_only_complete_safe_frames_and_json_clipping_discards() {
    let raw=b"data: {\"model\":\"actual-response\",\"token\":\"secret\"}\n\ndata: {\"unfinished\":\"sec";
    let result = redaction::redact(redaction::Kind::Sse, raw, false, true, 4096, &[], None);
    assert_eq!(result.state, "truncated");
    assert!(!result.complete);
    assert!(!String::from_utf8_lossy(&result.data).contains("unfinished"));
    assert!(!String::from_utf8_lossy(&result.data).contains("secret"));
    assert_eq!(result.observed.model.as_deref(), Some("actual-response"));
    let result = redaction::redact(
        redaction::Kind::Json,
        br#"{"valid":"prefix"}"#,
        true,
        true,
        4096,
        &[],
        None,
    );
    assert!(result.data.is_empty());
    assert_eq!(result.state, "size_limit");
}
#[test]
fn zero_retention_and_field_disabled_keep_no_payload_or_content_type_parameters() {
    for (fields, hours, expected) in [
        (vec![CaptureField::RequestBody], 0, "retention_zero"),
        (vec![], 24, "disabled"),
    ] {
        let f = fixture(fields, hours, 4096);
        f.context.add_secret("must-not-be-retained");
        let sink = f.context.body_sink(
            (None, CaptureField::RequestBody),
            &headers("application/json; token=CONTENT_TYPE_SECRET"),
            "decoded_after_request_decompression",
        );
        let raw = br#"{"model":"request","secret":"raw-not-retained"}"#;
        sink.feed(raw);
        sink.finish(true, None);
        assert!(f.context.lock().secrets.is_empty());
        let record = finish(&f.context);
        let blob = &record.blobs[&CaptureField::RequestBody];
        assert!(blob.data.is_empty());
        assert_eq!(blob.capture_state, expected);
        assert_eq!(blob.observed_bytes, raw.len() as u64);
        assert!(!blob.complete);
        assert_eq!(blob.content_type.as_deref(), Some("application/json"));
        assert_eq!(
            mime("text/CONTENT_TYPE_SECRET").as_deref(),
            Some("text/plain")
        );
    }
}
#[test]
fn observed_bytes_continue_after_shared_request_limit() {
    let f = fixture(vec![CaptureField::RequestBody], 24, 64);
    let sink = f.context.body_sink(
        (None, CaptureField::RequestBody),
        &headers("application/json"),
        "decoded_after_request_decompression",
    );
    sink.feed(&[b'x'; 128]);
    sink.feed(&[b'y'; 256]);
    sink.finish(true, None);
    let record = finish(&f.context);
    let blob = &record.blobs[&CaptureField::RequestBody];
    assert_eq!(blob.observed_bytes, 384);
    assert_eq!(blob.capture_state, "size_limit");
    assert!(blob.data.is_empty());
    assert!(!blob.complete);
}
#[test]
fn service_envelope_uses_historical_union_for_non_string_secrets() {
    let f = fixture(vec![CaptureField::UpstreamRequestBody], 24, 8192);
    let registration:Registration=serde_json::from_value(json!({"manifest":{"id":"service","name":"Service","version":"1","api_version":1,"config_schema":[{"key":"flag","type":"boolean","secret":true},{"key":"count","type":"number","secret":true},{"key":"object","type":"object","secret":true}],"capabilities":{"endpoints":["/v1/chat/completions"]}},"endpoint":"http://127.0.0.1:8765","expected_revision":0})).unwrap();
    f.context.0.plugins.register(registration).unwrap();
    let raw=br#"{"channel":{"config":{"flag":true,"count":17,"object":{"value":"PRIVATE"}}},"request":{"body":{"model":"wire"}}}"#;
    let result = redaction::redact(
        redaction::Kind::Json,
        raw,
        true,
        false,
        8192,
        &[],
        Some((f.context.0.plugins.as_ref(), "service")),
    );
    let value: Value = serde_json::from_slice(&result.data).unwrap();
    for key in ["flag", "count", "object"] {
        assert_eq!(value["channel"]["config"][key], "***");
    }
    assert!(!String::from_utf8_lossy(&result.data).contains("PRIVATE"));
}
#[tokio::test]
async fn replayable_request_body_is_not_changed_into_a_stream() {
    let f = fixture(
        vec![
            CaptureField::UpstreamRequestHeaders,
            CaptureField::UpstreamRequestBody,
        ],
        24,
        8192,
    );
    let attempt = f.context.begin_attempt(
        "core-r1",
        Some("channel"),
        Some("alias"),
        Some("wire"),
        false,
        Some("provider-token"),
        &["x-opaque".into()],
        None,
    );
    let client = reqwest::Client::new();
    let mut request = client
        .post("http://example.invalid/v1/chat/completions")
        .header("content-type", "application/json")
        .header("x-opaque", "custom-secret")
        .body(r#"{"model":"wire"}"#)
        .build()
        .unwrap();
    assert!(request.try_clone().is_some());
    attempt.request(&mut request);
    assert!(request.try_clone().is_some());
    assert_eq!(
        request.body().unwrap().as_bytes(),
        Some(br#"{"model":"wire"}"#.as_slice())
    );
    let record = finish(&f.context);
    let text = String::from_utf8_lossy(
        &record.attempt_blobs[&1][&CaptureField::UpstreamRequestHeaders].data,
    );
    assert!(!text.contains("custom-secret"));
}
#[derive(Clone, Debug)]
struct ExtensionMarker(&'static str);
#[tokio::test]
async fn response_roundtrip_preserves_url_status_headers_extensions_and_bytes() {
    let f = fixture(
        vec![
            CaptureField::UpstreamResponseHeaders,
            CaptureField::UpstreamResponseBody,
        ],
        24,
        8192,
    );
    let attempt = f.context.begin_attempt(
        "core-r1",
        Some("channel"),
        Some("alias"),
        Some("wire"),
        false,
        None,
        &[],
        None,
    );
    let raw = br#"{"model":"actual-response","choices":[{"message":{"content":"hello"}}]}"#;
    let original = reqwest::Response::from(
        Response::builder()
            .url("https://example.invalid/result".parse().unwrap())
            .status(202)
            .version(axum::http::Version::HTTP_11)
            .header("content-type", "application/json")
            .header("content-length", raw.len())
            .extension(ExtensionMarker("unchanged"))
            .body(reqwest::Body::from(raw.to_vec()))
            .unwrap(),
    );
    let observed = attempt.response(original);
    assert_eq!(observed.url().as_str(), "https://example.invalid/result");
    assert_eq!(observed.status(), 202);
    assert_eq!(observed.version(), axum::http::Version::HTTP_11);
    assert_eq!(
        observed.extensions().get::<ExtensionMarker>().unwrap().0,
        "unchanged"
    );
    assert_eq!(observed.content_length(), Some(raw.len() as u64));
    assert_eq!(observed.bytes().await.unwrap().as_ref(), raw);
    let record = finish(&f.context);
    assert_eq!(record.response_model.as_deref(), Some("actual-response"));
    assert!(record.usage.is_none());
    assert_eq!(record.upstream_complete, Some(true));
}
#[tokio::test]
async fn body_drop_is_cancelled_not_eof_and_does_not_wait_for_finalization() {
    let f = fixture(vec![CaptureField::ResponseBody], 24, 8192);
    let body = Body::from_stream(futures_util::stream::pending::<Result<Bytes, std::io::Error>>());
    let response = Response::builder()
        .header("content-type", "text/event-stream")
        .body(body)
        .unwrap();
    drop(f.context.response(response));
    let record = finish(&f.context);
    assert!(record.cancelled);
    assert_eq!(record.relay_complete, Some(false));
    assert!(record.incomplete);
    assert!(!record.blobs[&CaptureField::ResponseBody].complete);
}
#[tokio::test]
async fn response_roundtrip_keeps_transport_timeout_classification() {
    fn response() -> reqwest::Response {
        let stream = futures_util::stream::once(async {
            Err::<Bytes, _>(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "fixture timeout",
            ))
        });
        reqwest::Response::from(
            Response::builder()
                .header("content-type", "text/event-stream")
                .body(reqwest::Body::wrap_stream(stream))
                .unwrap(),
        )
    }
    let plain = response().bytes().await.unwrap_err();
    let f = fixture(vec![CaptureField::UpstreamResponseBody], 24, 8192);
    let attempt = f
        .context
        .begin_attempt("core-r1", None, None, None, true, None, &[], None);
    let observed = attempt.response(response()).bytes().await.unwrap_err();
    assert_eq!(plain.is_timeout(), observed.is_timeout());
    assert_eq!(plain.is_decode(), observed.is_decode());
    assert!(std::error::Error::source(&observed).is_some());
}
#[test]
fn attempts_are_independent_and_excess_metadata_is_explicitly_omitted() {
    let f = fixture(vec![CaptureField::UpstreamRequestBody], 24, 16384);
    for index in 0..(MAX_ATTEMPT_DETAILS + 2) {
        f.context.begin_attempt(
            &format!("core-r{index}"),
            Some(&"c".repeat(4096)),
            Some(&"m".repeat(4096)),
            Some("wire"),
            false,
            None,
            &[],
            None,
        );
    }
    let record = finish(&f.context);
    assert_eq!(record.attempt_count, 130);
    assert_eq!(record.omitted_attempts, 2);
    assert_eq!(record.attempts.len(), 128);
    assert_eq!(record.attempt_blobs.len(), 128);
    assert!(record.attempts[0]["channel"].as_str().unwrap().len() <= MAX_LABEL_BYTES + 3);
    assert_ne!(
        record.attempts[0]["attempt_id"],
        record.attempts[1]["attempt_id"]
    );
}
#[test]
fn finalizer_queue_time_does_not_inflate_record_duration() {
    let f = fixture(vec![], 0, 4096);
    let data = std::mem::take(&mut *f.context.lock());
    let started = Instant::now() - std::time::Duration::from_secs(10);
    let ended = started + std::time::Duration::from_millis(12);
    let record = FinalizeJob {
        state: data,
        logs: f.context.0.logs.clone(),
        plugins: f.context.0.plugins.clone(),
        started,
        ended,
        raw_limit: 4096,
    }
    .finalize(true, None);
    assert_eq!(record.duration_ms, Some(12.0));
}
#[test]
fn global_reservation_and_queue_fallback_release_raw_only() {
    let f = fixture(vec![CaptureField::ResponseBody], 24, 8192);
    assert!(CaptureReservation::acquire(MAX_CAPTURE_BYTES_GLOBAL + 1).is_none());
    let sink = f.context.body_sink(
        (None, CaptureField::ResponseBody),
        &headers("text/plain"),
        "relay",
    );
    sink.feed(b"known raw bytes");
    sink.finish(true, None);
    let data = std::mem::take(&mut *f.context.lock());
    let record = FinalizeJob {
        state: data,
        logs: f.context.0.logs.clone(),
        plugins: f.context.0.plugins.clone(),
        started: f.context.0.started,
        ended: Instant::now(),
        raw_limit: 8192,
    }
    .finalize(false, Some("capture_queue_full"));
    assert!(record.blobs[&CaptureField::ResponseBody].data.is_empty());
    assert_eq!(
        record.capture_failure.as_deref(),
        Some("capture_queue_full")
    );
    assert_eq!(
        record.capture_budget.as_ref().unwrap().bytes,
        METADATA_RESERVATION
    );
}
#[test]
fn svg_is_text_and_absent_usage_is_not_reported_as_zero() {
    assert_eq!(redaction::kind("image/svg+xml", ""), redaction::Kind::Text);
    let result = redaction::redact(
        redaction::Kind::Text,
        b"<svg><text>secret-value</text></svg>",
        true,
        false,
        4096,
        &["secret-value".into()],
        None,
    );
    assert!(result.complete);
    assert!(!String::from_utf8_lossy(&result.data).contains("secret-value"));
    assert!(redaction::usage_json(&crate::observability::usage::FactUsage::default()).is_none());
    let usage = crate::observability::usage::FactUsage::from_usage(Some(
        &json!({"prompt_tokens":0,"completion_tokens":0}),
    ));
    assert_eq!(redaction::usage_json(&usage).unwrap()["total_tokens"], 0);
}

#[test]
fn moderation_attempt_does_not_overwrite_client_model_or_stream() {
    let f = fixture(vec![], 0, 4096);
    f.context.lock().record.endpoint = Some("/v1/chat/completions".into());
    f.context.client_metadata("public-chat", true);
    let attempt = f.context.begin_attempt(
        "req-moderation-one",
        Some("safety-channel"),
        Some("safety-model"),
        Some("safety-wire"),
        false,
        None,
        &[],
        None,
    );
    attempt.tag_endpoint("/v1/moderations");
    let record = finish(&f.context);
    assert_eq!(record.requested_model.as_deref(), Some("public-chat"));
    assert!(record.stream);
    assert_eq!(record.attempt_count, 1);
    assert_eq!(record.attempts[0]["phase"], "moderation");
    assert_eq!(record.attempts[0]["sent_model"], "safety-wire");
    assert!(record.sent_model.is_none());
    assert!(record.channel.is_none());
}
#[test]
fn actual_prepared_sent_model_wins_over_legacy_statistical_guess() {
    let f = fixture(vec![], 0, 4096);
    f.context.begin_attempt(
        "req-r1",
        Some("channel"),
        Some("public"),
        Some("actual-wire"),
        false,
        None,
        &[],
        None,
    );
    f.context
        .record_request(&crate::storage::database::RequestStat {
            model: "public".into(),
            upstream_model: "legacy-original".into(),
            status: 200,
            ..Default::default()
        });
    let record = finish(&f.context);
    assert_eq!(record.sent_model.as_deref(), Some("actual-wire"));
}

// Capacity checks deliberately inspect Vec::capacity(), not only data lengths.
fn assert_capacity_budget(context: &LogContext) {
    let state = context.lock();
    let actual: usize = state.slots.values().map(|slot| slot.raw.capacity()).sum();
    let retained: usize = state.slots.values().map(|slot| slot.raw.len()).sum();
    assert_eq!(state.raw_capacity, actual);
    assert_eq!(state.retained_bytes, retained);
    assert_eq!(
        state.secret_bytes,
        state.secrets.iter().map(String::capacity).sum::<usize>()
    );
    assert!(actual + state.secret_bytes <= context.limit());
    assert!(CAPTURE_BYTES.load(Ordering::Relaxed) <= MAX_CAPTURE_BYTES_GLOBAL);
}

#[test]
fn small_chunks_have_bounded_amortized_growth_without_losing_observations() {
    let limit = 64 * 1024;
    let f = fixture(vec![CaptureField::RequestBody], 24, limit);
    let sink = f.context.body_sink(
        (None, CaptureField::RequestBody),
        &headers("text/plain"),
        "test",
    );
    let chunk = [b'x'; 128];
    let mut previous = 0;
    let mut changes = 0;
    for _ in 0..limit / chunk.len() {
        sink.feed(&chunk);
        assert_capacity_budget(&f.context);
        let state = f.context.lock();
        let capacity = state.slots[&(None, CaptureField::RequestBody)]
            .raw
            .capacity();
        if capacity != previous {
            changes += 1;
            previous = capacity;
        }
    }
    assert!(
        changes <= 8,
        "growth must be logarithmic rather than once per chunk"
    );
    sink.feed(b"overflow");
    sink.finish(true, None);
    let state = f.context.lock();
    let slot = &state.slots[&(None, CaptureField::RequestBody)];
    assert_eq!(slot.raw.len(), limit);
    assert_eq!(slot.observed, (limit + 8) as u64);
    assert!(slot.clipped && slot.eof);
}

#[test]
fn capacity_pressure_and_late_secrets_do_not_cause_premature_truncation() {
    let f = fixture(
        vec![CaptureField::RequestBody, CaptureField::ResponseBody],
        24,
        1536,
    );
    let request = f.context.body_sink(
        (None, CaptureField::RequestBody),
        &headers("text/plain"),
        "request",
    );
    let response = f.context.body_sink(
        (None, CaptureField::ResponseBody),
        &headers("text/plain"),
        "response",
    );
    request.feed(&[b'a'; 700]);
    response.feed(&[b'b'; 500]);
    assert_capacity_budget(&f.context);
    {
        let state = f.context.lock();
        assert!(state.raw_capacity > state.retained_bytes);
    }
    f.context
        .add_secret("0123456789012345678901234567890123456789");
    assert_capacity_budget(&f.context);
    assert!(!f.context.lock().raw_failed);
    request.feed(&[b'c'; 296]);
    assert_capacity_budget(&f.context);
    {
        let state = f.context.lock();
        assert_eq!(state.retained_bytes + state.secret_bytes, 1536);
        assert!(!state.slots[&(None, CaptureField::RequestBody)].clipped);
        assert_eq!(
            state.slots[&(None, CaptureField::ResponseBody)].raw.len(),
            500
        );
    }
    request.feed(b"one byte too many");
    request.finish(true, None);
    response.finish(false, Some("body_dropped_before_eof"));
    assert_capacity_budget(&f.context);
    let state = f.context.lock();
    assert_eq!(
        state.slots[&(None, CaptureField::RequestBody)].observed,
        1013
    );
    assert_eq!(state.record.relay_complete, Some(false));
    assert!(state.record.cancelled);
}

#[test]
fn headers_and_both_fail_closed_paths_maintain_capacity_ledger() {
    for reason in ["secret", "header_metadata"] {
        let f = fixture(
            vec![CaptureField::RequestHeaders, CaptureField::RequestBody],
            24,
            4096,
        );
        let mut source = headers("application/json");
        source.insert("authorization", "Bearer provider-key".parse().unwrap());
        f.context
            .headers((None, CaptureField::RequestHeaders), &source, &[]);
        let sink = f.context.body_sink(
            (None, CaptureField::RequestBody),
            &headers("text/plain"),
            "request",
        );
        sink.feed(&[b'x'; 1200]);
        assert_capacity_budget(&f.context);
        // Replacing an already-observed header map must release its old charge.
        f.context.headers(
            (None, CaptureField::RequestHeaders),
            &headers("text/plain"),
            &[],
        );
        assert_capacity_budget(&f.context);
        if reason == "secret" {
            f.context.add_secret(&"s".repeat(4096));
        } else {
            f.context.begin_attempt(
                "attempt",
                None,
                None,
                None,
                false,
                None,
                &["n".repeat(4097)],
                None,
            );
        }
        assert_capacity_budget(&f.context);
        {
            let state = f.context.lock();
            assert!(state.raw_failed);
            assert_eq!(state.raw_capacity, 0);
            assert_eq!(state.retained_bytes, 0);
        }
        sink.feed(b"still observed");
        assert_eq!(
            f.context.lock().slots[&(None, CaptureField::RequestBody)].observed,
            1214
        );
    }
}

#[test]
fn disabled_header_capture_still_supplies_secrets_to_captured_body() {
    let f = fixture(vec![CaptureField::RequestBody], 24, 8192);
    let mut source = HeaderMap::new();
    source.insert("authorization", "Bearer caller-key".parse().unwrap());
    source.insert("x-opaque", "declared-private".parse().unwrap());
    f.context.headers(
        (None, CaptureField::RequestHeaders),
        &source,
        &["x-opaque".into()],
    );
    let sink = f.context.body_sink(
        (None, CaptureField::RequestBody),
        &headers("application/json"),
        "request",
    );
    sink.feed(br#"{"message":"caller-key and declared-private"}"#);
    sink.finish(true, None);
    assert_capacity_budget(&f.context);
    let record = finish(&f.context);
    assert_eq!(
        record.blobs[&CaptureField::RequestHeaders].capture_state,
        "disabled"
    );
    let saved = String::from_utf8_lossy(&record.blobs[&CaptureField::RequestBody].data);
    assert!(!saved.contains("caller-key") && !saved.contains("declared-private"));
}

fn benchmark_headers() -> HeaderMap {
    let mut result = headers("application/json");
    for index in 0..8 {
        result.insert(
            format!("x-api-key-{index}")
                .parse::<axum::http::HeaderName>()
                .unwrap(),
            format!("Bearer provider-value-{index}").parse().unwrap(),
        );
    }
    result.insert("x-echo", "provider-value-3".parse().unwrap());
    result
}
fn count_lock_entries(context: &LogContext, operation: impl FnOnce()) -> usize {
    context.0.lock_count.store(0, Ordering::Relaxed);
    context.0.count_locks.store(true, Ordering::Relaxed);
    operation();
    context.0.count_locks.store(false, Ordering::Relaxed);
    context.0.lock_count.load(Ordering::Relaxed)
}

// Small test-only pre-optimization reference. The old path took one mutex per
// secret/composite-token insertion, cloned the secret set and built Value rows.
// The bounded secret checks stay shared so this does not weaken test security.
fn legacy_headers(context: &LogContext, source: &HeaderMap) {
    for (name, value) in source {
        if sensitive_header(name.as_str(), &[]) {
            if let Ok(value) = value.to_str() {
                context.lock().add_secret(value, context.limit());
                if let Some((scheme, token)) = value.split_once(' ') {
                    if matches!(scheme.to_ascii_lowercase().as_str(), "bearer" | "basic") {
                        context.lock().add_secret(token, context.limit());
                    }
                }
            }
        }
    }
    let observed = source
        .iter()
        .map(|(name, value)| name.as_str().len() as u64 + value.as_bytes().len() as u64)
        .sum();
    let mut state = context.lock();
    let secrets = state.secrets.clone();
    let available = context
        .limit()
        .saturating_sub(state.retained_bytes + state.secret_bytes);
    let slot = state
        .slots
        .get_mut(&(None, CaptureField::RequestHeaders))
        .unwrap();
    slot.observed = observed;
    slot.eof = true;
    slot.ended = true;
    slot.kind = redaction::Kind::Json;
    slot.content_type = Some("application/json".into());
    slot.representation = Some("parsed_http_header_map_name_value_octets".into());
    if !slot.collecting() {
        return;
    }
    let rows = source.iter().map(|(name, value)| json!({"name":name.as_str(),"value":if sensitive_header(name.as_str(),&[]){redaction::MASK.to_owned()}else{redaction::clean_text(value.to_str().unwrap(),&secrets)}})).collect::<Vec<_>>();
    let bytes = legacy_encode(&Value::Array(rows), available);
    let capacity = bytes.capacity();
    let len = bytes.len();
    slot.raw = bytes;
    slot.state = "collecting";
    state.retained_bytes += len;
    state.raw_capacity += capacity;
}
fn legacy_encode(value: &Value, limit: usize) -> Vec<u8> {
    struct Exact {
        data: Vec<u8>,
        limit: usize,
    }
    impl std::io::Write for Exact {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.data.len() + bytes.len() > self.limit {
                return Err(std::io::Error::other("limit"));
            }
            self.data
                .try_reserve_exact(bytes.len())
                .map_err(|_| std::io::Error::other("allocation"))?;
            self.data.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Exact {
        data: Vec::new(),
        limit,
    };
    serde_json::to_writer(&mut writer, value).unwrap();
    writer.data
}
fn clear_captured_slot(context: &LogContext, key: (Option<u32>, CaptureField)) {
    let mut state = context.lock();
    let slot = state.slots.get_mut(&key).unwrap();
    let len = slot.raw.len();
    let capacity = slot.raw.capacity();
    slot.raw = Vec::new();
    slot.observed = 0;
    slot.clipped = false;
    slot.ended = false;
    slot.eof = false;
    if slot.state == "collecting" {
        slot.state = "not_observed";
    }
    state.retained_bytes -= len;
    state.raw_capacity -= capacity;
}

#[test]
fn headers_use_one_lock_and_preserve_baseline_bytes_in_all_capture_modes() {
    let source = benchmark_headers();
    for (fields, hours) in [
        (vec![], 0),
        (vec![CaptureField::RequestHeaders], 0),
        (vec![CaptureField::RequestHeaders], 24),
    ] {
        let f = fixture(fields, hours, 64 * 1024);
        let baseline_locks = count_lock_entries(&f.context, || legacy_headers(&f.context, &source));
        let (baseline_data, observed, state_name) = {
            let state = f.context.lock();
            let slot = &state.slots[&(None, CaptureField::RequestHeaders)];
            (slot.raw.clone(), slot.observed, slot.state)
        };
        clear_captured_slot(&f.context, (None, CaptureField::RequestHeaders));
        let optimized_locks = count_lock_entries(&f.context, || {
            f.context
                .headers((None, CaptureField::RequestHeaders), &source, &[])
        });
        assert_eq!(baseline_locks, 17);
        assert_eq!(optimized_locks, 1);
        assert_capacity_budget(&f.context);
        let state = f.context.lock();
        let slot = &state.slots[&(None, CaptureField::RequestHeaders)];
        assert_eq!(slot.raw, baseline_data);
        assert_eq!(slot.observed, observed);
        assert_eq!(slot.state, state_name);
        if hours == 0 {
            assert!(state.secrets.is_empty());
            assert_eq!(state.raw_capacity, 0);
        }
    }
}

#[test]
fn finalized_serializer_capacity_is_tight_and_redaction_semantics_are_unchanged() {
    for size in [0, 1, 127, 1025, 4097] {
        let value = json!({"message":"x".repeat(size),"numbers":[0,1,2],"null":null});
        let expected = legacy_encode(&value, 16384);
        let encoded = redaction::encode(&value, 16384).unwrap();
        assert_eq!(encoded, expected);
        assert_eq!(encoded.capacity(), encoded.len());
        // The same serializer must accept an exact-size allowance and fail
        // cleanly one byte below it, regardless of its amortized growth steps.
        let exact = redaction::encode(&value, expected.len()).unwrap();
        assert_eq!(exact, expected);
        assert_eq!(exact.capacity(), expected.len());
        assert!(redaction::encode(&value, expected.len() - 1).is_err());
    }
    let input = b"data: {\"model\":\"wire\",\"token\":\"secret\"}\n\ndata: [DONE]\n\n";
    let result = redaction::redact(redaction::Kind::Sse, input, true, false, 4096, &[], None);
    assert!(result.complete);
    assert_eq!(result.data.capacity(), result.data.len());
    assert_eq!(result.observed.model.as_deref(), Some("wire"));
    assert_eq!(result.observed.terminal, Some(true));
    assert!(!String::from_utf8_lossy(&result.data).contains("secret"));
}

// No benchmark framework or service processes. Run both variants in one release
// test binary, e.g. cargo test --release capture_microbench -- --ignored
// --nocapture --test-threads=1. End-to-end release measurements remain separate.
#[test]
#[ignore = "focused release-only allocation and header hot-path microbenchmark"]
fn capture_microbench_growth_and_headers() {
    use std::hint::black_box;
    assert!(
        !cfg!(debug_assertions),
        "Run this A/B benchmark in --release for both variants"
    );
    let limit = 1024 * 1024;
    for chunk_size in [512, 8192] {
        let chunk = vec![b'x'; chunk_size];
        for amortized in [false, true] {
            let mut changes = 0usize;
            let mut peak = 0usize;
            let mut total = 0usize;
            let started = Instant::now();
            for _ in 0..32 {
                let mut bytes = Vec::new();
                while bytes.len() < limit {
                    let before = bytes.capacity();
                    let take = chunk.len().min(limit - bytes.len());
                    if amortized {
                        redaction::reserve_bounded(&mut bytes, take, limit).unwrap();
                    } else {
                        bytes.try_reserve_exact(take).unwrap();
                    }
                    bytes.extend_from_slice(black_box(&chunk[..take]));
                    changes += usize::from(bytes.capacity() != before);
                    peak = peak.max(bytes.capacity());
                    assert!(bytes.capacity() <= limit);
                }
                total += black_box(bytes.len());
            }
            println!(
                "{}",
                json!({"benchmark":"capture_growth","profile":"release","variant":if amortized{"bounded_amortized"}else{"baseline_exact"},"chunk_bytes":chunk_size,"runs":32,"payload_bytes":total,"capacity_changes":changes,"peak_actual_capacity":peak,"per_request_limit":limit,"elapsed_ns":started.elapsed().as_nanos()})
            );
        }
    }
    let source = benchmark_headers();
    for raw in [false, true] {
        let f = fixture(
            if raw {
                vec![CaptureField::RequestHeaders]
            } else {
                vec![]
            },
            if raw { 24 } else { 0 },
            64 * 1024,
        );
        for optimized in [false, true] {
            let run = || {
                if optimized {
                    f.context
                        .headers((None, CaptureField::RequestHeaders), &source, &[]);
                } else {
                    legacy_headers(&f.context, &source);
                }
            };
            let locks = count_lock_entries(&f.context, run);
            clear_captured_slot(&f.context, (None, CaptureField::RequestHeaders));
            let started = Instant::now();
            let mut peak = 0usize;
            let mut saved = 0usize;
            for _ in 0..2000 {
                run();
                {
                    let state = f.context.lock();
                    peak = peak.max(state.raw_capacity);
                    saved += black_box(state.retained_bytes);
                }
                clear_captured_slot(&f.context, (None, CaptureField::RequestHeaders));
            }
            println!(
                "{}",
                json!({"benchmark":"capture_headers","profile":"release","variant":if optimized{"single_lock_borrowed"}else{"baseline_lock_clone_value"},"raw_enabled":raw,"runs":2000,"lock_entries_per_header_map":locks,"lock_counting_during_timing":false,"peak_actual_capacity":peak,"saved_bytes_sum":saved,"per_request_limit":65536,"elapsed_ns":started.elapsed().as_nanos()})
            );
        }
    }
}
