use super::*;
use crate::plugins::tests::{registration, registry};
use crate::providers::types::{AttemptBody, DownstreamProtocol};
use axum::body::to_bytes;
use serde_json::Map;
use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;

fn manifest() -> Manifest {
    serde_json::from_str(include_str!(
        "../../../examples/plugins/managed-native/manifest.json"
    ))
    .unwrap()
}

fn provider(name: &str, groups: Value) -> Arc<Provider> {
    let raw = serde_json::from_value(json!({"name":name,"base_url":"https://not-used.invalid","engine":"plugin:example-native","api":"upstream-key","models":{"public":"native-text"},"preferences":{"groups":groups,"channel_plugin":"example-native","protocol_mode":"native","parameter_policy":{"channel":{"remove":["/vendorParameter"]}}}})).unwrap();
    crate::config::snapshot::runtime_provider(raw, Arc::new(AtomicUsize::new(0)))
}

#[test]
fn native_proxy_manifest_and_mount_are_explicit() {
    let manifest = manifest();
    manifest.validate().unwrap();
    assert!(manifest.capabilities.endpoints.is_empty());
    assert_eq!(
        mount("/native/A%2B%2F%E6%B5%8B%E8%AF%95/api/generate"),
        Some(("A+/测试".into(), "/api/generate"))
    );
    assert_eq!(
        mount("/native/A+B/api/files"),
        Some(("A+B".into(), "/api/files"))
    );
    for path in [
        "/admin/api/plugins",
        "/v1/messages",
        "/native//api",
        "/native/channel",
    ] {
        assert!(mount(path).is_none());
    }
    let native = manifest.native_proxy.as_ref().unwrap();
    assert!(native.route(&Method::POST, "/api/generate").is_some());
    assert!(native.route(&Method::GET, "/api/generate").is_none());
    assert!(native.route(&Method::GET, "/admin").is_none());
    assert!(native.route(&Method::GET, "/api/jobs/job-one").is_some());
    let path = "/api/models/native-text:generateContent";
    assert_eq!(
        native.route(&Method::POST, path).unwrap().path_model(path),
        Some("native-text")
    );
}

#[test]
fn native_proxy_query_credentials_are_removed_without_reserializing_business_parameters() {
    let mut manifest = manifest();
    manifest.native_proxy.as_mut().unwrap().client_auth = ClientAuth::Query { name: "key".into() };
    let mut request = Request::builder()
        .uri("/native/channel/api/generate?x=a%20b&key=caller&x=a+b&&flag&%6bey=second")
        .header("authorization", "Bearer conflicting")
        .header("x-miora-byok-provider", "other")
        .header("x-miora-byok-api-key", "private")
        .header("x-uni-api-provider", "other")
        .body(Body::empty())
        .unwrap();
    normalize_credentials(&manifest, &mut request).unwrap();
    assert_eq!(
        crate::routing::access::extract_api_key(request.headers()).as_deref(),
        Some("caller")
    );
    assert_eq!(request.uri().query(), Some("x=a%20b&x=a+b&&flag"));
    assert!(!request.headers().contains_key("x-miora-byok-api-key"));
    assert!(!request.headers().contains_key("x-uni-api-provider"));
}

#[test]
fn native_proxy_declared_header_and_bearer_credentials_take_precedence() {
    let mut manifest = manifest();
    for (auth, name, value, expected) in [
        (
            ClientAuth::Bearer,
            "Authorization",
            "bEaReR gateway",
            "gateway",
        ),
        (
            ClientAuth::Header {
                name: "X-Vendor-Key".into(),
            },
            "x-vendor-key",
            "header-key",
            "header-key",
        ),
    ] {
        manifest.native_proxy.as_mut().unwrap().client_auth = auth;
        let mut request = Request::builder()
            .uri("/native/channel/api/generate")
            .header("x-api-key", "unrelated")
            .header(name, value)
            .body(Body::empty())
            .unwrap();
        normalize_credentials(&manifest, &mut request).unwrap();
        assert_eq!(
            crate::routing::access::extract_api_key(request.headers()).as_deref(),
            Some(expected)
        );
    }
    assert!(normalize_credentials(&manifest, &mut Request::new(Body::empty())).is_err());
}

#[test]
fn native_proxy_forwards_business_headers_not_credentials_cookies_or_hop_headers() {
    let manifest = manifest();
    let mut headers = HeaderMap::new();
    for (name, value) in [
        ("authorization", "Bearer caller"),
        ("cookie", "session=private"),
        ("x-api-key", "caller"),
        ("x-goog-api-key", "caller"),
        ("x-miora-debug", "private"),
        ("x-uni-api-provider", "other"),
        ("connection", "x-hop"),
        ("x-hop", "private"),
        ("anthropic-beta", "native-feature"),
        ("content-type", "multipart/form-data; boundary=original"),
        ("idempotency-key", "original"),
        ("accept-encoding", "gzip"),
    ] {
        headers.insert(name, value.parse().unwrap());
    }
    let forwarded: HashMap<_, _> = manifest
        .native_proxy
        .as_ref()
        .unwrap()
        .headers(&headers, &manifest)
        .into_iter()
        .collect();
    assert_eq!(forwarded.len(), 4);
    assert_eq!(forwarded["anthropic-beta"], "native-feature");
    assert_eq!(forwarded["idempotency-key"], "original");
    assert_eq!(
        forwarded["content-type"],
        "multipart/form-data; boundary=original"
    );
    assert_eq!(forwarded["accept-encoding"], "gzip");
    headers.insert(
        "x-vendor-text",
        HeaderValue::from_bytes(b"caf\xe9").unwrap(),
    );
    assert!(manifest
        .native_proxy
        .as_ref()
        .unwrap()
        .headers(&headers, &manifest)
        .contains(&("x-vendor-text".into(), "café".into())));
}

#[test]
fn native_proxy_channel_selection_preserves_model_and_group_permissions() {
    use crate::config::snapshot::{ApiKey, Snapshot};
    let selected = provider("selected", json!(["private"]));
    let other = provider("other", json!(["private"]));
    let snapshot = Snapshot {
        revision: "native-test".into(),
        preferences: Default::default(),
        api_keys: Default::default(),
        api_key_order: Default::default(),
        providers: Arc::new(vec![selected.clone(), other]),
        providers_by_name: Default::default(),
        virtual_models: Default::default(),
        api_config: Arc::new(Value::Null),
    };
    let mut key = ApiKey {
        token: "caller".into(),
        model_rules: Arc::new(vec!["all".into()]),
        role: "user".into(),
        preferences: Arc::new(Map::from_iter([("groups".into(), json!(["private"]))])),
        weights: Default::default(),
        native_supported: true,
    };
    let matches = |key: &ApiKey| {
        crate::routing::planner::matching_providers(
            &snapshot,
            key,
            "public",
            32,
            None,
            None,
            "/native/selected/api/generate",
        )
        .unwrap()
    };
    assert_eq!(matches(&key).len(), 1);
    assert_eq!(matches(&key)[0].name.as_ref(), "selected");
    key.model_rules = Arc::new(vec!["not-allowed".into()]);
    assert!(matches(&key).is_empty());
    key.model_rules = Arc::new(vec!["all".into()]);
    key.preferences = Default::default();
    assert!(matches(&key).is_empty());
    key.preferences = Arc::new(Map::from_iter([("groups".into(), json!(["private"]))]));
    let mut blocked = (*selected).clone();
    blocked.excluded_endpoints = Arc::new(vec!["/api/generate".into()]);
    let snapshot = Snapshot {
        providers: Arc::new(vec![Arc::new(blocked)]),
        ..snapshot
    };
    assert!(crate::routing::planner::matching_providers(
        &snapshot,
        &key,
        "public",
        32,
        None,
        None,
        "/native/selected/api/generate"
    )
    .unwrap()
    .is_empty());
    let mut blocked = (*selected).clone();
    blocked.excluded_request_rules = Arc::new(vec![json!({"match":{"endpoint":"/api/generate"}})]);
    let snapshot = Snapshot {
        providers: Arc::new(vec![Arc::new(blocked)]),
        ..snapshot
    };
    assert!(crate::routing::planner::matching_providers(
        &snapshot,
        &key,
        "public",
        32,
        None,
        None,
        "/native/selected/api/generate"
    )
    .unwrap()
    .is_empty());
}

#[tokio::test]
async fn native_proxy_runtime_keeps_raw_bytes_and_ignores_generic_parameter_removal() {
    let registry = registry();
    let mut registration = registration(0, "1.0.0");
    registration.manifest = manifest();
    registration.config = serde_json::from_value(
        json!({"base_url":"https://provider.invalid","api_key":"configured-key"}),
    )
    .unwrap();
    registry.register(registration).unwrap();
    let provider = provider("channel", json!(["default"]));
    let manager = crate::transport::spool::SpoolManager::new(
        crate::runtime::resources::ResourceGovernor::unconstrained_for_test(),
    )
    .unwrap();
    for (path, method, content_type, raw) in [
        ("/api/generate", Method::POST, "application/json", &b" {\"model\": \"native-text\", \"vendorParameter\": [1,2], \"tools\":{\"native\":true}} "[..]),
        ("/api/files", Method::POST, "multipart/form-data; boundary=unchanged", &b"--unchanged\r\nraw\x00\xff"[..]),
        ("/api/jobs/job-one", Method::DELETE, "application/octet-stream", &b""[..]),
    ] {
        let spool = read_spooled_body(Body::from(raw.to_vec()), &manager, None, Some(raw.len() as u64), Duration::ZERO).await.unwrap_or_else(|_| panic!("fixture spool failed"));
        assert!(spool.identity.is_none());
        let input = PreparedInput { payload:Some(json!({"model":"public"})), replay:Some((spool.storage, spool.observation.clone())), observation:spool.observation, default_model:"public".into(), content_type:content_type.into() };
        let uri: Uri = format!("/native/channel{path}?vendor=a%20b&vendor=c+d").parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer gateway-secret".parse().unwrap());
        let prepared = registry.prepare(&provider, "scheduled-key", "public", "native-text", &method, &uri, uri.path(), &headers, &input, "native-test").unwrap();
        assert_eq!(prepared.downstream_protocol, DownstreamProtocol::NativeProxy);
        assert!(prepared.policy_report.trace.is_empty());
        let AttemptBody::PluginResource { envelope, storage, observation } = prepared.body else { panic!("raw resource required") };
        let envelope: Value = serde_json::from_slice(&envelope).unwrap();
        assert_eq!(envelope["request"]["format"], "native");
        assert_eq!(envelope["request"]["endpoint"], path);
        assert_eq!(envelope["request"]["method"], method.as_str());
        assert_eq!(envelope["request"]["query"], "vendor=a%20b&vendor=c+d");
        assert_eq!(envelope["channel"]["config"]["api_key"], "scheduled-key");
        assert!(!envelope.to_string().contains("gateway-secret"));
        assert!(envelope["request"].get("body").is_none());
        assert_eq!(to_bytes(storage.into_body(&observation).await.unwrap(), 4096).await.unwrap().as_ref(), raw);
    }
}
