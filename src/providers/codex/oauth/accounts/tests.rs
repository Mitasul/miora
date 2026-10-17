use super::*;
use crate::plugins::registry::{PluginRegistry, Registration, TrustPolicy};
use crate::providers::types::{AttemptBody, PreparedAttempt, PreparedInput};
use axum::http::{HeaderMap, Method, StatusCode};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Store(PathBuf);
impl Store {
    fn new() -> Self {
        static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
        Self(
            std::env::temp_dir()
                .join(format!(
                    "miora-core-oauth-{}-{}-{}",
                    std::process::id(),
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_nanos(),
                    SEQUENCE.fetch_add(1, Ordering::Relaxed)
                ))
                .join("tokens.json"),
        )
    }
    fn manager(&self) -> CodexOAuthManager {
        CodexOAuthManager {
            store_path: Arc::new(self.0.clone()),
            token_url: "no-network://token".into(),
            ..CodexOAuthManager::new()
        }
    }
}
impl Drop for Store {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let _ = std::fs::remove_dir(self.0.parent().unwrap());
    }
}

fn registration(revision: u64) -> Registration {
    serde_json::from_value(json!({
        "manifest":{"id":"shared-codex","name":"Codex images","version":format!("{}.0.0", revision + 1),"api_version":1,
            "auth":{"type":"core_oauth","provider":"codex"},
            "config_schema":[{"key":"base_url","type":"string","default":"https://example.invalid"},{"key":"account_id","type":"string","default":"stale-fixed-account"}],
            "capabilities":{"endpoints":["/v1/images/generations","/v1/images/edits","/v1/responses"],"streaming":true,"model_discovery":true,"request_formats":["json","form","multipart","binary"]},
            "native_proxy":{"client_auth":{"type":"bearer"},"routes":[{"path":"/images","methods":["POST"],"model":{"source":"fixed","value":"upstream"}}]}},
        "endpoint":format!("http://127.0.0.1:{}/v{}", 8765, revision + 1),
        "service_auth":{"type":"bearer","token":"service-secret"},"expected_revision":revision
    })).unwrap()
}
fn registry() -> PluginRegistry {
    let registry = PluginRegistry::open(
        None,
        TrustPolicy {
            allowed_origins: vec![],
            allow_loopback_http: true,
        },
    )
    .unwrap();
    registry.register(registration(0)).unwrap();
    registry
}
fn provider(engine: &str) -> Arc<crate::config::snapshot::Provider> {
    let raw = serde_json::from_value(json!({"name":"images","base_url":"https://example.invalid/v1","engine":engine,"api":"__channel_plugin__","models":{"public":"upstream"},"preferences":{}})).unwrap();
    crate::config::snapshot::runtime_provider(raw, Arc::new(AtomicUsize::new(0)))
}
fn input(stream: bool) -> PreparedInput {
    PreparedInput {
        payload: Some(json!({"model":"public","prompt":"draw","stream":stream})),
        replay: None,
        observation: Default::default(),
        default_model: String::new(),
        content_type: "application/json".into(),
    }
}
async fn import(manager: &CodexOAuthManager, account: &str, expiry: u64) -> String {
    let raw = format!("{account},initial-refresh-{account}");
    manager.record_imported_metadata(&raw, &json!({"access_token":format!("access-{account}"),"account_id":account,"expires_at":expiry,"email":format!("{account}@example.invalid")})).await.unwrap();
    manager.account_reference(&raw, "codex").await.unwrap()
}
async fn prepared(
    registry: &PluginRegistry,
    manager: &CodexOAuthManager,
    key: &str,
    input: &PreparedInput,
    path: &str,
) -> PreparedAttempt {
    registry
        .prepare_authenticated(
            manager,
            &provider("plugin:shared-codex"),
            key,
            "public",
            "upstream",
            &Method::POST,
            &path.parse().unwrap(),
            path,
            &HeaderMap::new(),
            input,
            "fixture",
        )
        .await
        .unwrap()
}
fn envelope(prepared: PreparedAttempt) -> Value {
    assert_eq!(prepared.headers["authorization"], "Bearer service-secret");
    assert!(!prepared.headers.contains_key("chatgpt-account-id"));
    let bytes = match prepared.body {
        AttemptBody::Json(bytes) => bytes,
        AttemptBody::PluginResource { envelope, .. } => envelope,
        _ => panic!("plugin envelope required"),
    };
    assert!(!String::from_utf8_lossy(&bytes).contains("initial-refresh"));
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(value["resolved_auth"].get("refresh_token").is_none());
    value
}

#[tokio::test]
async fn core_oauth_stable_records_are_shared_and_supplier_scoped() {
    let store = Store::new();
    let manager = store.manager();
    let reference = import(&manager, "account-a", unix_seconds() + 3600).await;
    assert!(reference.starts_with(CODEX_ACCOUNT_PREFIX));
    assert_eq!(
        manager
            .account_reference("account-a,initial-refresh-account-a", "codex")
            .await
            .unwrap(),
        reference
    );
    assert_eq!(
        manager
            .account_reference("account-a,another-reference", "codex")
            .await
            .unwrap(),
        reference
    );
    assert!(manager
        .account_reference("oauth:claude,token", "codex")
        .await
        .is_err());
    assert!(manager
        .resolve_core(&reference, "claude", None)
        .await
        .is_err());
    let accounts = manager.core_accounts().await.unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0]["account_ref"], reference);
    assert!(!serde_json::to_string(&accounts)
        .unwrap()
        .contains("access-account-a"));
    assert!(!serde_json::to_string(&accounts)
        .unwrap()
        .contains("initial-refresh"));
    let saved = manager.persisted.lock().await;
    let saved = saved.as_ref().unwrap();
    assert_eq!(
        saved
            .values()
            .filter(|entry| matches!(entry, SavedToken::Account(_)))
            .count(),
        1
    );
    assert!(matches!(
        &saved["account-a,initial-refresh-account-a"],
        SavedToken::Reference { .. }
    ));
}

#[tokio::test]
async fn core_oauth_balances_plugin_uses_the_same_selected_account_without_its_refresh_token() {
    let store = Store::new();
    let manager = store.manager();
    let reference = import(&manager, "account-b", unix_seconds() + 3600).await;
    let registry = registry();
    let manifest: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/plugins/balances/codex/manifest.json"
    )))
    .unwrap();
    registry.register(serde_json::from_value(json!({"manifest":manifest,"endpoint":"http://127.0.0.1:8766","expected_revision":0})).unwrap()).unwrap();
    let channel = provider("plugin:shared-codex");
    assert_eq!(
        registry.balance_plugin_id(&channel).unwrap().as_deref(),
        Some("balance-codex")
    );
    let snapshot = registry.snapshot();
    let envelope = PluginRegistry::balance_envelope(
        &manager,
        &channel,
        &reference,
        None,
        &snapshot.versions["balance-codex"],
        Some(&snapshot.versions["shared-codex"]),
    )
    .await
    .unwrap();
    assert_eq!(
        envelope["resolved_auth"]["access_token"],
        "access-account-b"
    );
    assert_eq!(envelope["resolved_auth"]["account_id"], "account-b");
    assert_eq!(envelope["resolved_auth"]["account_ref"], reference);
    assert_eq!(
        envelope["channel"]["config"]["base_url"],
        "https://example.invalid"
    );
    assert!(!envelope.to_string().contains("initial-refresh"));
    assert_eq!(manager.core_accounts().await.unwrap().len(), 1);
    let denied = registry
        .balance_authenticated(
            &manager,
            &channel,
            "core-oauth:codex:missing",
            None,
            "balance-codex",
        )
        .await
        .unwrap();
    assert_eq!(denied["status"], "auth_error");
}

#[tokio::test]
async fn core_oauth_reimport_keeps_the_record_and_old_channel_reference_on_latest_tokens() {
    let store = Store::new();
    let manager = store.manager();
    let reference = import(&manager, "account-a", unix_seconds() + 3600).await;
    manager.record_imported_metadata("account-a,reimported-refresh", &json!({"access_token":"reimported-access","account_id":"account-a","expires_at":unix_seconds() + 7200})).await.unwrap();
    assert_eq!(
        manager
            .account_reference("account-a,reimported-refresh", "codex")
            .await
            .unwrap(),
        reference
    );
    let old = manager
        .resolve("account-a,initial-refresh-account-a", None)
        .await
        .unwrap();
    assert_eq!(old.bearer, "reimported-access");
    assert_eq!(
        manager.persisted_refresh_token(&reference).await.as_deref(),
        Some("reimported-refresh")
    );
    let restarted = store.manager();
    assert_eq!(
        restarted
            .resolve_core(&reference, "codex", None)
            .await
            .unwrap()
            .access_token,
        "reimported-access"
    );
    assert_eq!(restarted.core_accounts().await.unwrap().len(), 1);
}

#[tokio::test]
async fn core_oauth_text_and_image_concurrency_refreshes_once_and_survives_restart() {
    let store = Store::new();
    let mut manager = store.manager();
    let reference = import(&manager, "account-a", unix_seconds() + 5).await;
    let fixture = Arc::new(RefreshFixture {
        calls: AtomicUsize::new(0),
        result: Ok(CachedToken {
            access_token: "rotated-access".into(),
            refresh_token: "rotated-refresh".into(),
            account_id: Some("account-a".into()),
            expires_at: Some(unix_seconds() + 3600),
            email: None,
            plan_type: None,
        }),
    });
    manager.refresh_fixture = Some(fixture.clone());
    let registry = registry();
    let payload = PreparedInput {
        payload: Some(
            json!({"model":"public","messages":[{"role":"user","content":"hi"}],"stream":true}),
        ),
        ..input(true)
    };
    let text_provider = provider("codex");
    let text_uri = "/v1/chat/completions".parse().unwrap();
    let headers = HeaderMap::new();
    let image_input = input(false);
    let (text, image) = tokio::join!(
        registry.prepare_authenticated(
            &manager,
            &text_provider,
            "account-a,initial-refresh-account-a",
            "public",
            "upstream",
            &Method::POST,
            &text_uri,
            "/v1/chat/completions",
            &headers,
            &payload,
            "text"
        ),
        prepared(
            &registry,
            &manager,
            &reference,
            &image_input,
            "/v1/images/generations"
        )
    );
    let text = text.unwrap();
    assert_eq!(text.headers["authorization"], "Bearer rotated-access");
    assert_eq!(text.headers["chatgpt-account-id"], "account-a");
    let image = envelope(image);
    assert_eq!(image["resolved_auth"]["access_token"], "rotated-access");
    assert_eq!(image["resolved_auth"]["account_id"], "account-a");
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
    let restarted = store.manager();
    let reloaded = restarted
        .resolve_core(&reference, "codex", None)
        .await
        .unwrap();
    assert_eq!(reloaded.access_token, "rotated-access");
    assert_eq!(reloaded.account_id, "account-a");
    assert_eq!(
        restarted
            .persisted_refresh_token("account-a,initial-refresh-account-a")
            .await
            .as_deref(),
        Some("rotated-refresh")
    );
}

#[tokio::test]
async fn core_oauth_round_robin_accounts_and_all_plugin_transports_keep_paired_metadata() {
    let store = Store::new();
    let manager = store.manager();
    let registry = registry();
    let first = import(&manager, "account-a", unix_seconds() + 3600).await;
    let second = import(&manager, "account-b", unix_seconds() + 3600).await;
    let runtime = crate::runtime::state::GatewayRuntime::new();
    let raw = serde_json::from_value(json!({"name":"images","base_url":"https://example.invalid","engine":"plugin:shared-codex","api":[first,second],"models":{"public":"upstream"},"preferences":{}})).unwrap();
    let pool = crate::config::snapshot::runtime_provider(raw, Arc::new(AtomicUsize::new(0)));
    for account in ["account-a", "account-b", "account-a"] {
        let crate::runtime::scheduling::ProviderKeySelection::Selected(reference) =
            runtime.select_provider_key(&pool, "upstream").await
        else {
            panic!("account selection required")
        };
        for stream in [false, true] {
            let wire = envelope(
                prepared(
                    &registry,
                    &manager,
                    &reference,
                    &input(stream),
                    "/v1/images/generations",
                )
                .await,
            );
            assert_eq!(
                wire["resolved_auth"]["access_token"],
                format!("access-{account}")
            );
            assert_eq!(wire["resolved_auth"]["account_id"], account);
            assert_eq!(wire["resolved_auth"]["account_ref"], reference.as_str());
            assert_eq!(wire["request"]["stream"], stream);
        }
    }
    let spool = crate::transport::spool::SpoolManager::new(
        crate::runtime::resources::ResourceGovernor::unconstrained_for_test(),
    )
    .unwrap();
    for (content_type, path, bytes) in [
        (
            "application/x-www-form-urlencoded",
            "/v1/images/edits",
            &b"model=public&prompt=draw"[..],
        ),
        (
            "multipart/form-data; boundary=test",
            "/v1/images/edits",
            &b"--test\r\n\r\nimage\r\n--test--\r\n"[..],
        ),
        ("image/png", "/v1/images/edits", &b"\x89PNG\x00\xff"[..]),
        (
            "application/octet-stream",
            "/native/images/images",
            &b"\x00\xffraw"[..],
        ),
    ] {
        let mut writer = spool
            .begin(None, Some(bytes.len() as u64), Duration::ZERO)
            .await
            .unwrap();
        writer
            .append(bytes::Bytes::copy_from_slice(bytes))
            .await
            .unwrap();
        let body = writer.finish().await.unwrap();
        let payload = PreparedInput {
            replay: Some((body.storage, body.observation)),
            content_type: content_type.into(),
            ..input(false)
        };
        let wire = envelope(prepared(&registry, &manager, &second, &payload, path).await);
        assert_eq!(wire["resolved_auth"]["account_id"], "account-b");
        assert_eq!(wire["resolved_auth"]["access_token"], "access-account-b");
        assert_eq!(wire["request"]["body_resource"]["size"], bytes.len());
    }
    let snapshot = registry.snapshot();
    let wire = PluginRegistry::discovery_envelope(
        &manager,
        &snapshot.versions["shared-codex"],
        "images",
        Default::default(),
        &second,
        None,
    )
    .await
    .unwrap();
    assert_eq!(wire["resolved_auth"]["account_id"], "account-b");
    assert_eq!(wire["resolved_auth"]["access_token"], "access-account-b");
    assert!(wire["resolved_auth"].get("refresh_token").is_none());
    assert_eq!(
        wire["channel"]["config"]["account_id"],
        "stale-fixed-account"
    );
}

#[tokio::test]
async fn core_oauth_plugin_updates_rollback_and_pins_do_not_change_account_links() {
    let store = Store::new();
    let manager = store.manager();
    let reference = import(&manager, "account-a", unix_seconds() + 3600).await;
    let registry = registry();
    let pinned = registry.pin();
    registry.register(registration(1)).unwrap();
    let old = envelope(
        prepared(
            &pinned,
            &manager,
            &reference,
            &input(false),
            "/v1/images/generations",
        )
        .await,
    );
    let new = envelope(
        prepared(
            &registry,
            &manager,
            &reference,
            &input(false),
            "/v1/images/generations",
        )
        .await,
    );
    assert_eq!(old["plugin"]["version"], "1.0.0");
    assert_eq!(new["plugin"]["version"], "2.0.0");
    assert_eq!(old["resolved_auth"], new["resolved_auth"]);
    registry.change("shared-codex", 2, None, Some(1)).unwrap();
    let rolled_back = envelope(
        prepared(
            &registry,
            &manager,
            &reference,
            &input(false),
            "/v1/images/generations",
        )
        .await,
    );
    assert_eq!(rolled_back["resolved_auth"], old["resolved_auth"]);
    assert_eq!(manager.core_accounts().await.unwrap().len(), 1);
}

#[tokio::test]
async fn core_oauth_revocation_is_an_auth_error_before_plugin_dispatch_and_images_are_not_replayed()
{
    let store = Store::new();
    let mut manager = store.manager();
    let reference = import(&manager, "account-a", unix_seconds() + 1).await;
    let fixture = Arc::new(RefreshFixture {
        calls: AtomicUsize::new(0),
        result: Err("OAuth authorization expired or revoked".into()),
    });
    manager.refresh_fixture = Some(fixture.clone());
    let registry = registry();
    let provider = provider("plugin:shared-codex");
    let error = registry
        .prepare_authenticated(
            &manager,
            &provider,
            &reference,
            "public",
            "upstream",
            &Method::POST,
            &"/v1/images/generations".parse().unwrap(),
            "/v1/images/generations",
            &HeaderMap::new(),
            &input(false),
            "revoked",
        )
        .await
        .err()
        .unwrap();
    assert_eq!(error.status(), StatusCode::UNAUTHORIZED);
    assert!(error.to_string().contains("expired or revoked"));
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
    assert!(registry.core_oauth_single_execution(
        &provider,
        &Method::POST,
        "/v1/images/generations"
    ));
    assert!(registry.core_oauth_single_execution(&provider, &Method::POST, "/v1/images/edits"));
    assert!(!registry.core_oauth_single_execution(&provider, &Method::POST, "/v1/responses"));
    assert!(manager
        .resolve_core("core-oauth:codex:missing", "codex", None)
        .await
        .is_err());
}

#[tokio::test]
async fn core_oauth_api_key_plugins_do_not_infer_oauth_from_credential_contents() {
    let store = Store::new();
    let manager = store.manager();
    let registry = PluginRegistry::open(
        None,
        TrustPolicy {
            allowed_origins: vec![],
            allow_loopback_http: true,
        },
    )
    .unwrap();
    let mut registration = registration(0);
    registration.manifest.auth.kind = "bearer".into();
    registration.manifest.auth.provider.clear();
    registration.manifest.auth.config_key = "credential".into();
    registration.manifest.config_schema.push(
        serde_json::from_value(
            json!({"key":"credential","type":"string","secret":true,"required":true}),
        )
        .unwrap(),
    );
    registry.register(registration).unwrap();
    for key in [
        "account,looks-like-refresh",
        "{\"token\":\"not-oauth\"}",
        "core-oauth:codex:not-an-account",
    ] {
        let attempt = prepared(
            &registry,
            &manager,
            key,
            &input(false),
            "/v1/images/generations",
        )
        .await;
        let AttemptBody::Json(bytes) = attempt.body else {
            panic!("JSON required")
        };
        let wire: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(wire["channel"]["config"]["credential"], key);
        assert!(wire.get("resolved_auth").is_none());
    }
    assert!(!store.0.exists());
}

#[test]
fn core_oauth_manifest_requires_explicit_supplier_and_exclusive_refresh_ownership() {
    let mut registration = registration(0);
    registration.manifest.validate().unwrap();
    registration.manifest.capabilities.oauth = true;
    assert!(registration.manifest.validate().is_err());
    registration.manifest.capabilities.oauth = false;
    registration.manifest.auth.provider = "claude".into();
    assert!(registration.manifest.validate().is_err());
    registration.manifest.auth.provider = "codex".into();
    registration.manifest.auth.config_key = "refresh_token".into();
    assert!(registration.manifest.validate().is_err());
}
