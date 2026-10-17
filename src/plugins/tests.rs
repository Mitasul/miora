use super::registry::{PluginRegistry, Registration, TrustPolicy};
use axum::body::{to_bytes, Body};
use axum::http::{HeaderMap, Method};
use serde_json::{json, Value};
use std::sync::{atomic::AtomicUsize, Arc};

fn trust() -> TrustPolicy {
    TrustPolicy {
        allowed_origins: vec![],
        allow_loopback_http: true,
    }
}
pub(super) fn registry() -> PluginRegistry {
    PluginRegistry::open(None, trust()).unwrap()
}
pub(super) fn registration(expected: u64, version: &str) -> Registration {
    serde_json::from_value(json!({"manifest":{"id":"echo","name":"Echo","version":version,"api_version":1,"config_schema":[{"key":"prefix","label":"Prefix","type":"string","default":"Hello"},{"key":"credential","type":"string","secret":true}],"auth":{"type":"none"},"capabilities":{"endpoints":["/v1/chat/completions","/v1/responses"],"streaming":true,"model_discovery":true},"models":[{"id":"echo-model"}]},"endpoint":"http://127.0.0.1:8765","config":{"credential":"channel-secret"},"service_auth":{"type":"bearer","token":"service-secret"},"expected_revision":expected})).unwrap()
}
fn provider() -> Arc<crate::config::snapshot::Provider> {
    let raw = serde_json::from_value(json!({"name":"test","base_url":"https://not-used.invalid","engine":"plugin:echo","api":"__channel_plugin__","models":{"public-model":"echo-model"},"preferences":{"channel_plugin":"echo"}})).unwrap();
    crate::config::snapshot::runtime_provider(raw, Arc::new(AtomicUsize::new(0)))
}
fn input(stream: bool) -> crate::providers::types::PreparedInput {
    crate::providers::types::PreparedInput {
        payload: Some(
            json!({"model":"public-model","messages":[{"role":"user","content":"test"}],"stream":stream}),
        ),
        replay: None,
        observation: Default::default(),
        default_model: String::new(),
        content_type: "application/json".into(),
    }
}
fn prepare(registry: &PluginRegistry, stream: bool) -> crate::providers::types::PreparedAttempt {
    let mut headers = HeaderMap::new();
    headers.insert("authorization", "Bearer caller-secret".parse().unwrap());
    headers.insert("cookie", "admin_session=private".parse().unwrap());
    registry
        .prepare(
            &provider(),
            "__channel_plugin__",
            "public-model",
            "echo-model",
            &Method::POST,
            &"/v1/chat/completions".parse().unwrap(),
            "/v1/chat/completions",
            &headers,
            &input(stream),
            "req-test",
        )
        .unwrap()
}
#[test]
fn update_disable_rollback_keep_pinned_request_revision() {
    let reg = registry();
    reg.register(registration(0, "1.0.0")).unwrap();
    let pinned = reg.pin();
    reg.register(registration(1, "2.0.0")).unwrap();
    assert_eq!(pinned.snapshot().versions["echo"].revision, 1);
    assert_eq!(reg.snapshot().versions["echo"].revision, 2);
    let old = prepare(&pinned, false);
    let crate::providers::types::AttemptBody::Json(bytes) = old.body else {
        panic!()
    };
    let payload: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(payload["plugin"]["revision"], 1);
    reg.change("echo", 2, Some(false), None).unwrap();
    assert!(!reg.snapshot().versions.contains_key("echo"));
    assert!(reg.list()["plugins"][0]["versions"][0]["draining"]
        .as_bool()
        .unwrap());
    let _still_works = prepare(&pinned, true);
    reg.change("echo", 3, Some(true), None).unwrap();
    reg.change("echo", 4, None, Some(1)).unwrap();
    assert_eq!(reg.snapshot().versions["echo"].revision, 1);
    assert_eq!(reg.list()["plugins"][0]["revision"], 5);
}
#[test]
fn invalid_update_cas_and_secret_masks_preserve_current() {
    let reg = registry();
    reg.register(registration(0, "1")).unwrap();
    let cached = reg.snapshot();
    assert!(reg.register(registration(0, "2")).is_err());
    assert!(Arc::ptr_eq(&cached, &reg.snapshot()));
    let mut bad = registration(1, "2");
    bad.endpoint = "http://169.254.169.254/metadata".into();
    assert!(reg.register(bad).is_err());
    assert!(Arc::ptr_eq(&cached, &reg.snapshot()));
    let mut bad = registration(1, "2");
    bad.config.insert("unknown".into(), json!(1));
    assert!(reg.register(bad).is_err());
    assert!(Arc::ptr_eq(&cached, &reg.snapshot()));
    let mut masked = registration(1, "2");
    masked.config.insert("credential".into(), json!("***"));
    masked.service_auth.token = "***".into();
    reg.register(masked).unwrap();
    assert_eq!(
        reg.snapshot().versions["echo"].config["credential"],
        "channel-secret"
    );
    let list = reg.list().to_string();
    assert!(!list.contains("channel-secret"));
    assert!(!list.contains("service-secret"));
    assert_eq!(reg.snapshot().versions["echo"].revision, 2);
}
#[test]
fn trust_is_exact_and_no_url_credentials_or_redirect_destinations() {
    let policy = TrustPolicy {
        allowed_origins: vec!["https://plugins.example".into()],
        allow_loopback_http: false,
    };
    for endpoint in [
        "http://127.0.0.1:8765",
        "https://plugins.example.attacker.test",
        "https://user:pass@plugins.example",
        "https://plugins.example?token=secret",
        "https://plugins.example/#fragment",
        "file:///tmp/plugin",
        "https://plugins.example:8443",
    ] {
        assert!(policy.validate(endpoint).is_err(), "{endpoint}");
    }
    assert!(policy.validate("https://plugins.example/service").is_ok());
    assert!(trust().validate("http://127.0.0.1:8765").is_ok());
    assert!(trust().validate("http://localhost:8765").is_err());
}
#[test]
fn interceptor_chain_is_ordered_separate_and_credential_safe() {
    let reg = registry();
    reg.register(registration(0, "1")).unwrap();
    let old = reg.pin();
    let chain=serde_json::from_value(json!([{"id":"second","enabled":true,"order":20,"set_headers":{"x-demo":"second"}},{"id":"first","enabled":true,"order":10,"set_headers":{"x-demo":"first"}}])).unwrap();
    reg.update_interceptors(0, chain).unwrap();
    assert_eq!(prepare(&reg, false).headers["x-demo"], "second");
    assert!(!prepare(&old, false).headers.contains_key("x-demo"));
    let forbidden = serde_json::from_value(
        json!([{"id":"bad","enabled":true,"set_headers":{"authorization":"stolen"}}]),
    )
    .unwrap();
    assert!(reg.update_interceptors(1, forbidden).is_err());
    assert_eq!(reg.interceptors()["revision"], 1);
}
#[tokio::test]
async fn lease_survives_stream_and_releases_on_eof_or_cancel() {
    use futures_util::StreamExt;
    let reg = registry();
    reg.register(registration(0, "1")).unwrap();
    let prepared = prepare(&reg, true);
    let lease = prepared.plugin_lease.unwrap();
    let version = Arc::downgrade(&lease.snapshot.versions["echo"]);
    reg.register(registration(1, "2")).unwrap();
    let body = lease.retain(Body::from_stream(futures_util::stream::iter([
        Ok::<_, std::io::Error>("data: first\n\n"),
        Ok("data: [DONE]\n\n"),
    ])));
    let mut stream = body.into_data_stream();
    assert!(version.strong_count() > 1);
    assert_eq!(reg.list()["plugins"][0]["versions"][0]["draining"], true);
    assert!(stream.next().await.unwrap().is_ok());
    assert!(version.strong_count() > 1);
    assert!(stream.next().await.unwrap().is_ok());
    assert!(stream.next().await.is_none());
    assert_eq!(version.strong_count(), 1);
    assert_eq!(reg.list()["plugins"][0]["versions"][0]["leases"], 0);
    assert_eq!(reg.list()["plugins"][0]["versions"][0]["draining"], false);
    let lease = prepare(&reg, true).plugin_lease.unwrap();
    let current_snapshot = Arc::downgrade(&lease.snapshot);
    let current = Arc::downgrade(&lease.snapshot.versions["echo"]);
    let cached_version_references = current.strong_count();
    let body = lease.retain(Body::from("unread"));
    assert!(current_snapshot.strong_count() > 1);
    drop(body);
    // The publication now owns one Snapshot/Version reference while idle.
    // Cancellation must still release every external stream reference.
    assert_eq!(current_snapshot.strong_count(), 1);
    assert_eq!(current.strong_count(), cached_version_references);
    assert_eq!(reg.list()["plugins"][0]["versions"][1]["leases"], 0);

    let lease = prepare(&reg, true).plugin_lease.unwrap();
    let retired = Arc::downgrade(&lease.snapshot);
    let body = lease.retain(Body::from("cancelled while disabled"));
    reg.change("echo", 2, Some(false), None).unwrap();
    assert_eq!(reg.list()["plugins"][0]["versions"][1]["draining"], true);
    drop(body);
    assert_eq!(retired.strong_count(), 0);
    assert_eq!(reg.list()["plugins"][0]["versions"][1]["leases"], 0);
    assert_eq!(reg.list()["plugins"][0]["versions"][1]["draining"], false);
}
#[test]
fn persistence_roundtrip_and_failed_write_do_not_publish() {
    let path = std::env::temp_dir().join(format!(
        "miora-plugin-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let reg = PluginRegistry::open(Some(path.clone()), trust()).unwrap();
    reg.register(registration(0, "1")).unwrap();
    reg.register(registration(1, "2")).unwrap();
    reg.change("echo", 2, None, Some(1)).unwrap();
    let reopened = PluginRegistry::open(Some(path.clone()), trust()).unwrap();
    let restored = reopened.snapshot();
    assert!(Arc::ptr_eq(&restored, &reopened.snapshot()));
    assert_eq!(restored.versions["echo"].revision, 1);
    assert_eq!(reopened.list()["plugins"][0]["revision"], 3);
    assert!(PluginRegistry::open(
        Some(path.clone()),
        TrustPolicy {
            allowed_origins: vec![],
            allow_loopback_http: false
        }
    )
    .is_err());
    std::fs::remove_dir_all(&path).unwrap();
    std::fs::write(&path, b"block directory creation").unwrap();
    let retained = reg.snapshot();
    assert!(reg.register(registration(3, "bad-persist")).is_err());
    assert!(Arc::ptr_eq(&retained, &reg.snapshot()));
    assert!(reg.change("echo", 3, Some(false), None).is_err());
    assert!(Arc::ptr_eq(&retained, &reg.snapshot()));
    let chain = serde_json::from_value(
        json!([{"id":"blocked","enabled":true,"set_headers":{"x-safe":"value"}}]),
    )
    .unwrap();
    assert!(reg.update_interceptors(0, chain).is_err());
    assert!(Arc::ptr_eq(&retained, &reg.snapshot()));
    assert_eq!(reg.interceptors()["revision"], 0);
    assert_eq!(reg.snapshot().versions["echo"].manifest.version, "1");
    std::fs::remove_file(&path).unwrap();
}
#[test]
fn rollback_is_allowed_at_history_limit() {
    let reg = registry();
    let mut pins = Vec::new();
    for n in 0..100 {
        reg.register(registration(n, &n.to_string())).unwrap();
        pins.push(reg.pin());
    }
    assert!(reg.register(registration(100, "overflow")).is_err());
    reg.change("echo", 100, None, Some(1)).unwrap();
    assert_eq!(reg.snapshot().versions["echo"].revision, 1);
}
#[test]
fn execution_envelope_does_not_forward_gateway_credentials() {
    let reg = registry();
    reg.register(registration(0, "1")).unwrap();
    let prepared = prepare(&reg, false);
    assert_eq!(prepared.url, "http://127.0.0.1:8765/execute");
    assert_eq!(prepared.headers["authorization"], "Bearer service-secret");
    assert!(!prepared.headers.contains_key("cookie"));
    let crate::providers::types::AttemptBody::Json(bytes) = prepared.body else {
        panic!()
    };
    let text = String::from_utf8(bytes).unwrap();
    assert!(!text.contains("caller-secret"));
    assert!(!text.contains("private"));
    assert!(!text.contains("service-secret"));
    let payload: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(payload["request"]["body"]["model"], "echo-model");
    assert_eq!(payload["channel"]["config"]["credential"], "channel-secret");
}
#[tokio::test]
async fn real_http_model_discovery_and_sse_execute() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        for index in 0..2 {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let header_end = loop {
                let mut buf = [0; 1024];
                let n = socket.read(&mut buf).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
                if let Some(pos) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    break pos + 4;
                }
            };
            let headers = String::from_utf8_lossy(&bytes[..header_end]).to_lowercase();
            let length: usize = headers
                .lines()
                .find_map(|line| {
                    line.strip_prefix("content-length:")
                        .map(|v| v.trim().parse().unwrap())
                })
                .unwrap_or(0);
            while bytes.len() < header_end + length {
                let mut buf = [0; 1024];
                let n = socket.read(&mut buf).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
            }
            assert!(headers.contains("authorization: bearer service-secret"));
            assert!(!headers.contains("caller-secret"));
            let payload: Value =
                serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap();
            assert_eq!(payload["plugin"]["revision"], 1);
            let (kind, body) = if index == 0 {
                assert!(headers.starts_with("post /models "));
                (
                    "application/json",
                    "{\"models\":[{\"id\":\"echo-model\",\"name\":\"Echo\"}]}",
                )
            } else {
                assert!(headers.starts_with("post /execute "));
                assert_eq!(payload["request"]["stream"], true);
                (
                    "text/event-stream",
                    "data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\ndata: [DONE]\n\n",
                )
            };
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        }
    });
    let reg = registry();
    let mut registration = registration(0, "1");
    registration.endpoint = format!("http://{addr}");
    reg.register(registration).unwrap();
    assert_eq!(
        reg.models("echo", "test", Default::default())
            .await
            .unwrap()["models"][0]["id"],
        "echo-model"
    );
    let prepared = prepare(&reg, true);
    let crate::providers::types::AttemptBody::Json(bytes) = prepared.body else {
        panic!()
    };
    let response = reg
        .client
        .request(prepared.method, prepared.url)
        .headers(prepared.headers)
        .body(bytes)
        .send()
        .await
        .unwrap();
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let body = prepared
        .plugin_lease
        .unwrap()
        .retain(Body::from_stream(response.bytes_stream()));
    let bytes = to_bytes(body, 8192).await.unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("[DONE]"));
    server.join().unwrap();
}
#[tokio::test]
async fn management_handler_limits_methods_and_reports_conflict() {
    let reg = registry();
    reg.register(registration(0, "1")).unwrap();
    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/admin/api/plugins/echo/disable")
        .body(Body::from(r#"{"expected_revision":0}"#))
        .unwrap();
    let response = super::api::handle(&reg, request, &[]).await;
    assert_eq!(response.status(), 409);
    let request = axum::http::Request::builder()
        .method("DELETE")
        .uri("/admin/api/plugins/echo")
        .body(Body::empty())
        .unwrap();
    assert_eq!(super::api::handle(&reg, request, &[]).await.status(), 405);
}

#[test]
fn removed_secret_field_stays_redacted() {
    let reg = registry();
    reg.register(registration(0, "1")).unwrap();
    let channel = serde_json::from_value(json!({"credential":"channel-specific-secret"})).unwrap();
    let mut next = registration(1, "2");
    next.manifest
        .config_schema
        .retain(|f| f.key != "credential");
    next.config.remove("credential");
    reg.register(next).unwrap();
    assert_eq!(
        reg.redact_channel_config("echo", &channel)["credential"],
        "***"
    );
}

#[test]
fn rollback_keeps_union_of_historical_secret_fields_masked() {
    let reg = registry();
    let mut first = registration(0, "1");
    first
        .manifest
        .config_schema
        .iter_mut()
        .find(|f| f.key == "credential")
        .unwrap()
        .secret = false;
    reg.register(first).unwrap();
    reg.register(registration(1, "2")).unwrap();
    reg.change("echo", 2, None, Some(1)).unwrap();
    let channel = serde_json::from_value(json!({"credential":"new-secret-after-upgrade"})).unwrap();
    assert_eq!(
        reg.redact_channel_config("echo", &channel)["credential"],
        "***"
    );
    assert_eq!(reg.list()["plugins"][0]["config"]["credential"], "***");
}

#[tokio::test]
async fn failed_health_cannot_publish_unreachable_or_wrong_identity_candidate() {
    use std::io::{Read, Write};
    let reg = registry();
    let unused = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = unused.local_addr().unwrap();
    drop(unused);
    let mut initial = registration(0, "1");
    initial.endpoint = format!("http://{address}");
    reg.register(initial).unwrap();
    let mut unreachable = registration(1, "2");
    unreachable.endpoint = format!("http://{address}");
    let cached = reg.snapshot();
    assert!(reg.register_checked(unreachable).await.is_err());
    assert!(Arc::ptr_eq(&cached, &reg.snapshot()));
    assert_eq!(reg.list()["plugins"][0]["revision"], 1);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        loop {
            let mut buf = [0; 1024];
            let n = socket.read(&mut buf).unwrap();
            assert!(n > 0);
            request.extend_from_slice(&buf[..n]);
            if request.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let headers = String::from_utf8_lossy(&request).to_lowercase();
        assert!(headers.starts_with("get /health "));
        assert!(headers.contains("authorization: bearer service-secret"));
        assert!(!headers.contains("channel-secret"));
        let body = r#"{"ok":true,"api_version":1,"plugin_id":"wrong","version":"2"}"#;
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    });
    let mut candidate = registration(1, "2");
    candidate.endpoint = format!("http://{address}");
    assert!(reg
        .register_checked(candidate)
        .await
        .unwrap_err()
        .contains("identity"));
    assert_eq!(reg.snapshot().versions["echo"].manifest.version, "1");
    assert!(Arc::ptr_eq(&cached, &reg.snapshot()));
    server.join().unwrap();
    reg.change_checked("echo", 1, Some(false), None)
        .await
        .unwrap();
    let disabled = reg.snapshot();
    assert!(disabled.versions.is_empty());
    assert!(reg
        .change_checked("echo", 2, Some(true), None)
        .await
        .is_err());
    assert!(Arc::ptr_eq(&disabled, &reg.snapshot()));
    assert!(reg.change_checked("echo", 2, None, Some(1)).await.is_err());
    assert!(Arc::ptr_eq(&disabled, &reg.snapshot()));
}

#[test]
fn snapshot_arc_reused_until_a_successful_generation_change() {
    let reg = registry();
    let empty = reg.snapshot();
    assert!(Arc::ptr_eq(&empty, &reg.snapshot()));
    let before_registration = reg.pin();
    assert!(Arc::ptr_eq(&empty, &before_registration.snapshot()));

    reg.register(registration(0, "1")).unwrap();
    let registered = reg.snapshot();
    assert!(!Arc::ptr_eq(&empty, &registered));
    assert!(Arc::ptr_eq(&registered, &reg.snapshot()));
    assert!(before_registration.snapshot().versions.is_empty());
    let pinned = reg.pin();
    let repinned = pinned.pin();
    assert!(Arc::ptr_eq(&registered, &repinned.snapshot()));

    let chain = serde_json::from_value(
        json!([{"id":"cache-test","enabled":true,"set_headers":{"x-cache-test":"new"}}]),
    )
    .unwrap();
    reg.update_interceptors(0, chain).unwrap();
    let with_interceptors = reg.snapshot();
    assert!(!Arc::ptr_eq(&registered, &with_interceptors));
    assert!(registered.interceptors.is_empty());
    assert_eq!(with_interceptors.interceptors.len(), 1);

    reg.change("echo", 1, Some(false), None).unwrap();
    let disabled = reg.snapshot();
    assert!(!Arc::ptr_eq(&with_interceptors, &disabled));
    assert!(!disabled.versions.contains_key("echo"));
    reg.change("echo", 2, Some(true), None).unwrap();
    let enabled = reg.snapshot();
    assert!(!Arc::ptr_eq(&disabled, &enabled));
    assert_eq!(enabled.versions["echo"].revision, 1);

    reg.register(registration(3, "2")).unwrap();
    let replaced = reg.snapshot();
    assert!(!Arc::ptr_eq(&enabled, &replaced));
    assert_eq!(replaced.versions["echo"].revision, 4);
    reg.change("echo", 4, None, Some(1)).unwrap();
    let rolled_back = reg.snapshot();
    assert!(!Arc::ptr_eq(&replaced, &rolled_back));
    assert_eq!(rolled_back.versions["echo"].revision, 1);
    assert!(Arc::ptr_eq(&registered, &pinned.snapshot()));
    assert!(Arc::ptr_eq(&registered, &repinned.pin().snapshot()));
    assert!(pinned.snapshot().interceptors.is_empty());

    assert!(reg.change("echo", 4, Some(false), None).is_err());
    assert!(reg.change("echo", 5, None, Some(999)).is_err());
    let stale_chain = serde_json::from_value(json!([])).unwrap();
    assert!(reg.update_interceptors(0, stale_chain).is_err());
    assert!(Arc::ptr_eq(&rolled_back, &reg.snapshot()));
}

#[test]
fn shared_publication_is_not_an_idle_lease_and_retired_pins_drain() {
    let reg = registry();
    reg.register(registration(0, "1")).unwrap();
    assert_eq!(
        reg.list()["plugins"][0]["lease_count_kind"],
        "snapshot_references"
    );
    assert_eq!(reg.list()["plugins"][0]["versions"][0]["leases"], 0);
    let first = reg.pin();
    let second = first.pin();
    assert_eq!(reg.list()["plugins"][0]["versions"][0]["leases"], 2);
    reg.change("echo", 1, Some(false), None).unwrap();
    assert_eq!(reg.list()["plugins"][0]["versions"][0]["draining"], true);
    drop(first);
    assert_eq!(reg.list()["plugins"][0]["versions"][0]["leases"], 1);
    drop(second);
    assert_eq!(reg.list()["plugins"][0]["versions"][0]["leases"], 0);
    assert_eq!(reg.list()["plugins"][0]["versions"][0]["draining"], false);
    reg.change("echo", 2, Some(true), None).unwrap();
    assert_eq!(reg.list()["plugins"][0]["versions"][0]["leases"], 0);
}

#[test]
fn weak_upgrade_in_publication_window_is_tracked_without_an_initial_pin() {
    use std::cell::RefCell;
    use std::rc::Rc;

    let reg = registry();
    reg.register(registration(0, "1")).unwrap();
    let weak = Arc::downgrade(&reg.snapshot());
    // Only the registry owns the old snapshot when publication begins.
    assert_eq!(weak.strong_count(), 1);
    let retained = Rc::new(RefCell::new(None));
    let target = retained.clone();
    let probe = weak.clone();
    super::registry::before_next_publication(move || {
        // Reproduce an existing Weak upgrading after retirement bookkeeping,
        // but before the old Store owner is replaced. No scheduler race needed.
        assert_eq!(probe.strong_count(), 1);
        *target.borrow_mut() = Some(probe.upgrade().unwrap());
    });

    reg.change("echo", 1, Some(false), None).unwrap();
    let held = retained.borrow_mut().take().unwrap();
    assert_eq!(held.versions["echo"].revision, 1);
    assert!(reg.snapshot().versions.is_empty());
    assert_eq!(reg.list()["plugins"][0]["versions"][0]["leases"], 1);
    assert_eq!(reg.list()["plugins"][0]["versions"][0]["draining"], true);
    drop(held);
    assert!(weak.upgrade().is_none());
    assert_eq!(reg.list()["plugins"][0]["versions"][0]["leases"], 0);
    assert_eq!(reg.list()["plugins"][0]["versions"][0]["draining"], false);
}

#[tokio::test]
async fn successful_health_probe_cannot_overwrite_a_newer_publication() {
    use std::io::{Read, Write};
    let reg = Arc::new(registry());
    reg.register(registration(0, "1")).unwrap();
    let old = reg.pin();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (arrived, arrival) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            let mut bytes = [0; 1024];
            let size = socket.read(&mut bytes).unwrap();
            assert!(size > 0);
            request.extend_from_slice(&bytes[..size]);
        }
        arrived.send(()).unwrap();
        released
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let body = r#"{"ok":true,"api_version":1,"plugin_id":"echo","version":"probed"}"#;
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    });
    let mut candidate = registration(1, "probed");
    candidate.endpoint = format!("http://{address}");
    let checking = reg.clone();
    let check = tokio::spawn(async move { checking.register_checked(candidate).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), arrival)
        .await
        .unwrap()
        .unwrap();
    reg.register(registration(1, "winner")).unwrap();
    let winner = reg.snapshot();
    release.send(()).unwrap();
    assert_eq!(check.await.unwrap().unwrap_err(), "revision_conflict");
    assert!(Arc::ptr_eq(&winner, &reg.snapshot()));
    assert_eq!(old.snapshot().versions["echo"].manifest.version, "1");
    assert_eq!(winner.versions["echo"].manifest.version, "winner");
    server.join().unwrap();
}

#[test]
#[ignore = "manual release microbenchmark; reports pin latency, not gateway throughput"]
fn benchmark_cached_snapshot_pin() {
    const ITERATIONS: u32 = 100_000;
    for plugins in [0, 1, 128] {
        let reg = registry();
        for index in 0..plugins {
            let mut plugin = registration(0, "bench");
            plugin.manifest.id = format!("bench-{index}");
            reg.register(plugin).unwrap();
        }
        let original = reg.snapshot();
        for _ in 0..1000 {
            std::hint::black_box(reg.pin());
        }
        let start = std::time::Instant::now();
        for _ in 0..ITERATIONS {
            let pinned = std::hint::black_box(reg.pin());
            std::hint::black_box(pinned.snapshot());
        }
        let elapsed = start.elapsed();
        assert!(Arc::ptr_eq(&original, &reg.snapshot()));
        println!("cached_snapshot_pin plugins={plugins} iterations={ITERATIONS} total_us={} ns_per_iteration={:.1}", elapsed.as_micros(), elapsed.as_nanos() as f64 / f64::from(ITERATIONS));
    }
}
