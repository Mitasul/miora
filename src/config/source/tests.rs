use crate::config::compiler::compile_snapshot_bytes;
use crate::config::compiler::infer_engine;
use crate::config::discovery::discovered_model_ids;
use crate::config::discovery::provider_models_url;
use serde_json::{json, Value};

#[test]
fn compiles_python_compatible_model_mappings_and_paid_state() {
    let raw = br#"
providers:
  - provider: codex-a
    base_url: https://example.com/v1/responses
    engine: codex
    api: secret-upstream
    model:
      - upstream-sol: gpt-5.6-sol
api_keys:
  - api: client-key
    model:
      - codex-a/*
    preferences:
      rate_limit: 10/min
"#;
    let bytes = compile_snapshot_bytes(raw, false).unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        value["providers"][0]["models"]["gpt-5.6-sol"],
        "upstream-sol"
    );
    assert_eq!(value["providers"][0]["model_order"][0], "gpt-5.6-sol");
    assert_eq!(value["api_keys"][0]["native_paid_state_safe"], true);
    assert!(!value["revision"].as_str().unwrap().is_empty());
}

#[test]
fn preserves_provider_capabilities_and_video_provider_routes() {
    let raw = br#"
providers:
  - provider: p
    base_url: https://example.com/v1/chat/completions
    api: upstream
    model: [m]
    tools: true
    image: false
    preferences:
      api_key_schedule_algorithm: smart_round_robin
video_providers:
  - name: callxyq
    adapter: callxyq
    base_url: https://api.callxyq.xyz
    models:
      sora-2: sora-2
    routes:
      create_task: /v1/videos
api_keys:
  - api: parent
    model: [child/*]
  - api: child
    model: [callxyq/sora-2]
"#;
    let value: Value = serde_json::from_slice(&compile_snapshot_bytes(raw, true).unwrap()).unwrap();
    let p = value["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "p")
        .unwrap();
    assert_eq!(p["preferences"]["tools"], true);
    assert_eq!(p["preferences"]["image"], false);
    assert!(value["providers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["name"] == "callxyq"));
    assert_eq!(
        value["api_keys"][0]["preferences"]["__route_graph"][0],
        "child/*"
    );
}

#[test]
fn preserves_provider_exclude_request_rules() {
    let raw = br#"
providers:
  - provider: codex-a
    base_url: https://example.com/v1/responses
    engine: codex
    api: secret-upstream
    model:
      - codex-auto-review: gpt-5.6-luna
    exclude_request_rules:
      - match:
          endpoint: /v1/responses
          request_model: gpt-5.6-luna
          upstream_model: codex-auto-review
          reasoning_effort: [max]
        reason: unsupported_reasoning_effort
api_keys:
  - api: client-key
    model: [codex-a/*]
"#;
    let value: Value = serde_json::from_slice(&compile_snapshot_bytes(raw, true).unwrap()).unwrap();
    assert_eq!(
        value["providers"][0]["exclude_request_rules"][0]["match"]["reasoning_effort"][0],
        "max"
    );
}

#[test]
fn paid_key_is_admitted_to_native_database_balance_checks() {
    let raw = br#"
providers:
  - provider: p
    base_url: https://example.com/v1/responses
    api: upstream
    model: [m]
api_keys:
  - api: client
    model: [p/m]
    preferences:
      credits: 1.5
"#;
    let bytes = compile_snapshot_bytes(raw, false).unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["api_keys"][0]["native_paid_state_safe"], true);
}

#[test]
fn expands_nested_api_key_model_rules_and_infers_engines() {
    let raw = br#"
providers:
  - provider: claude-a
    base_url: https://example.com/v1/messages
    api: upstream
    model: [claude-opus]
api_keys:
  - api: child-key
    model: [claude-a/*]
  - api: parent-key
    model: [child-key/*]
"#;
    let bytes = compile_snapshot_bytes(raw, true).unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["providers"][0]["engine"], "claude");
    assert_eq!(value["api_keys"][1]["model_rules"][0], "claude-a/*");
}

#[test]
fn automatic_engine_uses_url_without_model_name_fallbacks() {
    for (base, expected) in [
        ("https://example.com/v1/responses", "codex"),
        ("https://example.com/proxy/v1/responses/?version=1", "codex"),
        (" https://example.com/V1/RESPONSES ", "codex"),
        ("https://example.com/v1/responses-preview", "gpt"),
        ("https://example.com/v1?redirect=/v1/responses", "gpt"),
        ("https://example.com/v1/chat/completions", "gpt"),
        ("https://example.com/v1", "gpt"),
        ("https://example.com/v1/messages", "claude"),
        ("https://example.com/claude/v1/responses", "claude"),
        ("https://generativelanguage.googleapis.com/v1beta", "gemini"),
        ("https://aiplatform.googleapis.com/v1/responses", "vertex"),
        (
            "https://bedrock-runtime.us-east-1.amazonaws.com/v1/responses",
            "aws",
        ),
        ("https://api.cohere.com/v1/responses", "cohere"),
        ("https://example.azure.com/v1/responses", "azure"),
        (
            "https://example.databricks.com/v1/responses",
            "azure-databricks",
        ),
        ("https://example.workers.dev/v1/responses", "cloudflare"),
        ("https://api.typesafe.ai/v1/responses", "typesafe"),
        (
            "https://example.com/doubao/v1/responses",
            "doubao-translation",
        ),
    ] {
        assert_eq!(infer_engine(base), expected, "{base}");
    }
    for model in [
        "claude-opus",
        "gemini-pro",
        "gpt-6-luna",
        "codex-review",
        "grok",
        "glm",
        "deepseek",
        "custom",
    ] {
        for (base, explicit, expected) in [
            ("https://example.com/v1/responses", None, "codex"),
            ("https://example.com/v1/responses", Some("gpt"), "gpt"),
            ("https://example.com/v1", None, "gpt"),
        ] {
            let mut provider =
                json!({"provider":"automatic","base_url":base,"api":"upstream","model":[model]});
            if let Some(engine) = explicit {
                provider["engine"] = json!(engine);
            }
            let config = json!({"providers":[provider],"api_keys":[{"api":"client","model":["automatic/*"]}]});
            let bytes =
                compile_snapshot_bytes(&serde_json::to_vec(&config).unwrap(), true).unwrap();
            let compiled: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(
                compiled["providers"][0]["engine"], expected,
                "{model} / {base}"
            );
            assert_eq!(
                compiled["providers"][0]["engine_mode"],
                if explicit.is_some() {
                    "explicit"
                } else {
                    "auto"
                }
            );
        }
    }
}

#[test]
fn preserves_operator_gateway_for_project_backed_provider() {
    let raw = br#"
providers:
  - provider: vertex-through-gateway
    base_url: https://gateway.example/proxy/https://aiplatform.googleapis.com/
    project_id: project-a
    api: upstream
    model: [gemini]
api_keys:
  - api: client
    model: [vertex-through-gateway/*]
"#;
    let bytes = compile_snapshot_bytes(raw, true).unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        value["providers"][0]["base_url"],
        "https://gateway.example/proxy/https://aiplatform.googleapis.com/"
    );
}

#[test]
fn model_discovery_normalizes_openai_and_gemini_shapes() {
    assert_eq!(
        provider_models_url("https://example.com/v1/chat/completions")
            .unwrap()
            .as_str(),
        "https://example.com/v1/models"
    );
    assert_eq!(
        discovered_model_ids(&json!({
            "models":[
                {"name":"models/gemini-2.5-pro"},
                {"name":"models/gemini-2.5-flash"},
                {"name":"models/gemini-2.5-pro"}
            ]
        })),
        vec!["gemini-2.5-flash", "gemini-2.5-pro"]
    );
}

#[test]
fn credential_only_special_providers_receive_native_route_keys() {
    let raw = br#"
providers:
  - provider: bedrock
    base_url: https://bedrock-runtime.us-east-1.amazonaws.com
    engine: aws
    aws_access_key: AKIA_TEST
    aws_secret_key: secret
    model: [anthropic.claude]
  - provider: vertex
    base_url: https://aiplatform.googleapis.com
    engine: vertex-claude
    client_email: svc@example.com
    private_key: key
    project_id: project-a
    model: [claude]
api_keys:
  - api: client
    model: [bedrock/*, vertex/*]
"#;
    let value: Value = serde_json::from_slice(&compile_snapshot_bytes(raw, true).unwrap()).unwrap();
    assert_eq!(value["providers"][0]["api"], "AKIA_TEST");
    assert_eq!(value["providers"][1]["api"], "__vertex_oauth__");
    assert_eq!(value["providers"][0]["aws_secret_key"], "secret");
}

async fn isolated_publisher(label: &str) -> (super::RuntimeConfigPublisher, std::path::PathBuf) {
    let directory = std::env::temp_dir().join(format!(
        "miora-source-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    tokio::fs::create_dir_all(&directory).await.unwrap();
    let source = directory.join("api.json");
    tokio::fs::write(&source, br#"{"providers":[],"api_keys":[]}"#)
        .await
        .unwrap();
    let publisher = super::RuntimeConfigPublisher {
        source: super::RuntimeConfigSource::File(source),
        client: reqwest::Client::new(),
        snapshot_path: std::sync::Arc::new(directory.join("snapshot.json")),
        database_disabled: true,
        discovery_cache: Default::default(),
    };
    publisher.publish().await.unwrap();
    (publisher, directory)
}

#[tokio::test]
async fn compare_and_swap_allows_only_one_concurrent_config_update() {
    let (publisher, directory) = isolated_publisher("cas").await;
    let metadata = tokio::fs::metadata(directory.join("api.json"))
        .await
        .unwrap();
    let revision = super::file_revision(&metadata).unwrap();
    let first = json!({"preferences":{"request_limit":1}});
    let second = json!({"preferences":{"request_limit":2}});
    let (a, b) = tokio::join!(
        publisher.apply_patch_revision(&first, &revision),
        publisher.apply_patch_revision(&second, &revision),
    );
    assert_ne!(a.is_ok(), b.is_ok());
    let rejected = if a.is_err() { a } else { b };
    assert_eq!(rejected.unwrap_err(), "configuration revision conflict");
    let config: Value =
        serde_yaml::from_slice(&tokio::fs::read(directory.join("api.json")).await.unwrap())
            .unwrap();
    let snapshot: Value = serde_json::from_slice(
        &tokio::fs::read(directory.join("snapshot.json"))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(snapshot["preferences"], config["preferences"]);
    tokio::fs::remove_dir_all(directory).await.unwrap();
}

#[tokio::test]
async fn failed_snapshot_publication_restores_the_source() {
    let (publisher, directory) = isolated_publisher("rollback").await;
    let original = tokio::fs::read(directory.join("api.json")).await.unwrap();
    tokio::fs::remove_file(directory.join("snapshot.json"))
        .await
        .unwrap();
    tokio::fs::create_dir(directory.join("snapshot.json"))
        .await
        .unwrap();
    assert!(publisher
        .apply_patch(&json!({"preferences":{"request_limit":3}}))
        .await
        .is_err());
    assert_eq!(
        original,
        tokio::fs::read(directory.join("api.json")).await.unwrap()
    );
    tokio::fs::remove_dir_all(directory).await.unwrap();
}

#[tokio::test]
async fn invalid_candidate_does_not_change_source_or_snapshot() {
    let (publisher, directory) = isolated_publisher("invalid").await;
    let original = tokio::fs::read(directory.join("api.json")).await.unwrap();
    let snapshot = tokio::fs::read(directory.join("snapshot.json"))
        .await
        .unwrap();
    assert!(publisher
        .apply_patch(&json!({"providers":"not-an-array"}))
        .await
        .is_err());
    assert_eq!(
        original,
        tokio::fs::read(directory.join("api.json")).await.unwrap()
    );
    assert_eq!(
        snapshot,
        tokio::fs::read(directory.join("snapshot.json"))
            .await
            .unwrap()
    );
    tokio::fs::remove_dir_all(directory).await.unwrap();
}

#[tokio::test]
async fn gateway_features_virtual_model_updates_replace_and_remove_saved_definitions() {
    let (publisher, directory) = isolated_publisher("virtual-models").await;
    publisher.apply_patch(&json!({
        "providers":[{"provider":"primary","base_url":"https://example.test/v1","api":"key","model":["real"]}],
        "virtual_models":{"first":[{"provider":"primary","model":"real"}],"second":[{"virtual":"first"}]}
    })).await.unwrap();
    let source = directory.join("api.json");
    let metadata = tokio::fs::metadata(&source).await.unwrap();
    publisher
        .apply_patch_revision(
            &json!({"virtual_models":{"remaining":[{"provider":"primary","model":"real"}]}}),
            &super::file_revision(&metadata).unwrap(),
        )
        .await
        .unwrap();
    let saved: Value = serde_yaml::from_slice(&tokio::fs::read(&source).await.unwrap()).unwrap();
    assert_eq!(saved["virtual_models"].as_object().unwrap().len(), 1);
    assert!(saved["virtual_models"].get("remaining").is_some());
    let snapshot: Value = serde_json::from_slice(
        &tokio::fs::read(directory.join("snapshot.json"))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        snapshot["preferences"]["__virtual_models"]
            .as_object()
            .unwrap()
            .len(),
        1
    );
    publisher
        .apply_patch(&json!({"virtual_models":{}}))
        .await
        .unwrap();
    let saved: Value = serde_yaml::from_slice(&tokio::fs::read(&source).await.unwrap()).unwrap();
    assert!(saved["virtual_models"].as_object().unwrap().is_empty());
    let snapshot: Value = serde_json::from_slice(
        &tokio::fs::read(directory.join("snapshot.json"))
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(snapshot["preferences"].get("__virtual_models").is_none());
    tokio::fs::remove_dir_all(directory).await.unwrap();
}
