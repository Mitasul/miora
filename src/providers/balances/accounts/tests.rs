use super::*;
use crate::plugins::registry::{PluginRegistry, Registration, TrustPolicy};

fn registry() -> PluginRegistry {
    PluginRegistry::open(
        None,
        TrustPolicy {
            allowed_origins: vec![],
            allow_loopback_http: true,
        },
    )
    .unwrap()
}
fn provider(engine: &str, base_url: &str, preferences: Value) -> std::sync::Arc<Provider> {
    crate::config::snapshot::runtime_provider(serde_json::from_value(json!({"name":"existing-channel", "engine":engine,
        "base_url":base_url,"api":"existing-key","models":{"alias":"real-model"},"preferences":preferences})).unwrap(), Default::default())
}
fn register(registry: &PluginRegistry, id: &str, targets: Value) {
    let registration: Registration = serde_json::from_value(json!({"manifest":{"id":id,"name":id,"version":"1.0.0","api_version":1,
        "auth":{"type":"none"},"capabilities":{"endpoints":[],"balance_query":true},"balance_targets":targets},
        "endpoint":"http://127.0.0.1:8765","expected_revision":0})).unwrap();
    registry.register(registration).unwrap();
}

#[test]
fn balances_no_installed_plugins_means_no_supplier_fallbacks() {
    let registry = registry();
    for (engine, base) in [
        ("codex", "https://chatgpt.com/backend-api/codex"),
        ("gpt", "https://image.novelai.net"),
        ("gpt", "https://api.deepseek.com"),
        ("gpt", "https://api.kimi.com/coding/v1"),
        ("gpt", "https://unknown.invalid/v1"),
    ] {
        let channel = provider(engine, base, json!({"balance_query":true}));
        assert_eq!(registry.balance_plugin_id(&channel).unwrap(), None);
        assert_eq!(channel.api_keys[0], "existing-key");
        assert_eq!(channel.models["alias"], "real-model");
    }
}

#[test]
fn balances_targets_require_exact_origins_and_path_boundaries() {
    let registry = registry();
    register(
        &registry,
        "account",
        json!([{"origins":["https://example.invalid"],"path_prefix":"/coding"}]),
    );
    for path in ["/coding", "/coding/v1", "/coding/v1/messages"] {
        assert_eq!(
            registry
                .balance_plugin_id(&provider(
                    "gpt",
                    &format!("https://example.invalid{path}"),
                    json!({})
                ))
                .unwrap()
                .as_deref(),
            Some("account")
        );
    }
    for base in [
        "https://example.invalid/coding-fake",
        "https://example.invalid/v1",
        "https://example.invalid.evil/coding/v1",
        "http://example.invalid/coding/v1",
    ] {
        assert_eq!(
            registry
                .balance_plugin_id(&provider("gpt", base, json!({})))
                .unwrap(),
            None
        );
    }
}

#[test]
fn balances_shipped_plugins_register_and_select_without_core_supplier_rules() {
    let registry = registry();
    for source in [
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/plugins/balances/codex/manifest.json"
        )),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/plugins/balances/novelai/manifest.json"
        )),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/plugins/balances/deepseek/manifest.json"
        )),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/plugins/balances/moonshot/manifest.json"
        )),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/plugins/balances/kimi-plan/manifest.json"
        )),
    ] {
        let manifest: Value = serde_json::from_str(source).unwrap();
        registry.register(serde_json::from_value(json!({"manifest":manifest,"endpoint":"http://127.0.0.1:8765","expected_revision":0})).unwrap()).unwrap();
    }
    for (engine, base, expected) in [
        (
            "codex",
            "https://proxy.invalid/v1/responses",
            "balance-codex",
        ),
        (
            "gpt",
            "https://image.novelai.net/ai/generate-image",
            "balance-novelai",
        ),
        (
            "claude",
            "https://api.deepseek.com/anthropic",
            "balance-deepseek",
        ),
        ("gpt", "https://api.moonshot.cn/v1", "balance-moonshot"),
        ("gpt", "https://api.moonshot.ai/v1", "balance-moonshot"),
        (
            "claude",
            "https://api.kimi.com/coding/v1",
            "balance-kimi-plan",
        ),
    ] {
        assert_eq!(
            registry
                .balance_plugin_id(&provider(engine, base, json!({})))
                .unwrap()
                .as_deref(),
            Some(expected)
        );
    }
}

#[test]
fn balances_explicit_selection_never_falls_back_when_missing_or_disabled() {
    let registry = registry();
    register(&registry, "account", json!([]));
    let channel = provider(
        "gpt",
        "https://proxy.invalid/v1",
        json!({"balance_plugin":"account"}),
    );
    assert_eq!(
        registry.balance_plugin_id(&channel).unwrap().as_deref(),
        Some("account")
    );
    registry.change("account", 1, Some(false), None).unwrap();
    assert!(registry.balance_plugin_id(&channel).is_err());
    assert!(registry
        .balance_plugin_id(&provider(
            "gpt",
            "https://proxy.invalid",
            json!({"balance_plugin":"missing"})
        ))
        .is_err());
}

#[test]
fn balances_multiple_matching_plugins_require_an_explicit_selection() {
    let registry = registry();
    for id in ["one", "two"] {
        register(&registry, id, json!([{"engines":["example"]}]));
    }
    assert!(registry
        .balance_plugin_id(&provider("example", "https://example.invalid", json!({})))
        .is_err());
    assert_eq!(
        registry
            .balance_plugin_id(&provider(
                "example",
                "https://example.invalid",
                json!({"balance_plugin":"two"})
            ))
            .unwrap()
            .as_deref(),
        Some("two")
    );
}

#[test]
fn balances_channel_plugins_keep_their_own_declared_hook() {
    let registry = registry();
    register(&registry, "channel", json!([]));
    register(&registry, "other", json!([{"engines":["plugin:channel"]}]));
    let channel = provider(
        "plugin:channel",
        "https://example.invalid",
        json!({"channel_plugin":"channel"}),
    );
    assert_eq!(
        registry.balance_plugin_id(&channel).unwrap().as_deref(),
        Some("channel")
    );
    assert_eq!(channel.engine.as_ref(), "plugin:channel");
}

#[test]
fn balances_projection_keeps_timestamps_and_availability_without_secrets() {
    let value: AccountBalance=serde_json::from_value(json!({"amount":0,"currency":"CNY","available":false,
        "expires_at":"2026-10-31T12:00:00+08:00","windows":[{"label":"limit","reset_at":"2026-10-18T00:00:00Z"}],
        "access_token":"do-not-store","refresh_token":"do-not-store","config":{"api_key":"do-not-store"}})).unwrap();
    let projected = value.into_value("plugin:account").unwrap();
    assert_eq!(projected["available"], false);
    assert_eq!(projected["amount"], 0.0);
    assert_eq!(projected["windows"][0]["reset_at"], "2026-10-18T00:00:00Z");
    assert!(!projected.to_string().contains("do-not-store"));
    assert!(AccountBalance::default()
        .into_value("plugin:account")
        .is_err());
}
