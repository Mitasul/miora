use super::*;
use crate::config::snapshot::runtime_provider;
use crate::plugins::registry::TrustPolicy;
use std::sync::{atomic::AtomicUsize, Arc};

fn provider(engine: &str, url: &str, policy: Option<Value>) -> Arc<Provider> {
    let mut preferences = serde_json::Map::new();
    if let Some(policy) = policy {
        preferences.insert("parameter_policy".into(), policy);
    }
    provider_with_preferences(engine, url, preferences)
}
fn provider_with_preferences(
    engine: &str,
    url: &str,
    preferences: serde_json::Map<String, Value>,
) -> Arc<Provider> {
    let raw=serde_json::from_value(json!({"name":"fixture","base_url":url,"engine":engine,"api":"fixture-secret","models":{"public":"wire"},"preferences":preferences})).unwrap();
    runtime_provider(raw, Arc::new(AtomicUsize::new(0)))
}
fn prepare(provider: &Provider, endpoint: &str, payload: Value) -> Result<PreparedAttempt, String> {
    let input = PreparedInput {
        payload: Some(payload),
        replay: None,
        observation: Default::default(),
        default_model: String::new(),
        content_type: "application/json".into(),
    };
    crate::plugins::builtin::build_attempt(
        provider,
        "fixture-secret",
        "public",
        "wire",
        &Method::POST,
        &endpoint.parse().unwrap(),
        endpoint,
        &HeaderMap::new(),
        &input,
        "parameter-test",
    )
}
fn body(attempt: &PreparedAttempt) -> Value {
    match &attempt.body {
        AttemptBody::Json(bytes) => serde_json::from_slice(bytes).unwrap(),
        _ => panic!("expected JSON"),
    }
}
fn chat() -> Value {
    json!({"model":"public","messages":[{"role":"system","content":"old"},{"role":"developer","content":"developer must stay"},{"role":"user","content":"hello"}],"stream":false})
}

#[test]
fn explicit_native_mode_keeps_requested_wire_protocol_and_preview_consistent() {
    for engine in ["gpt", "openrouter"] {
        let preferences = serde_json::from_value(json!({"protocol_mode":"native"})).unwrap();
        let channel = provider_with_preferences(engine, "https://upstream.example/v1", preferences);
        let input =
            json!({"model":"public","input":"hello","stream":false,"vendor_field":{"keep":true}});
        let attempt = prepare(&channel, "/v1/responses", input.clone()).unwrap();
        assert_eq!(attempt.url, "https://upstream.example/v1/responses");
        assert_eq!(body(&attempt)["input"], "hello");
        assert!(body(&attempt).get("messages").is_none());
        assert_eq!(body(&attempt)["vendor_field"], input["vendor_field"]);
        assert_eq!(
            capabilities(&channel, "/v1/responses")["profile"],
            "responses"
        );
        let registry = crate::plugins::PluginRegistry::open(
            None,
            TrustPolicy {
                allowed_origins: vec![],
                allow_loopback_http: false,
            },
        )
        .unwrap();
        let preview = prepare_preview(
            &registry,
            &channel,
            "public",
            "wire",
            "/v1/responses",
            input,
        )
        .unwrap();
        assert_eq!(body(&preview), body(&attempt));
        assert_eq!(preview.url, attempt.url);
    }
    let preferences = serde_json::from_value(json!({"protocol_mode":"native"})).unwrap();
    let channel =
        provider_with_preferences("gpt", "https://upstream.example/v1/responses", preferences);
    let attempt = prepare(&channel, "/v1/chat/completions", chat()).unwrap();
    assert!(body(&attempt).get("messages").is_some());
    assert!(body(&attempt).get("input").is_none());
    let preferences = serde_json::from_value(json!({"protocol_mode":"native"})).unwrap();
    let channel = provider_with_preferences(
        "codex",
        "https://upstream.example/v1/responses",
        preferences,
    );
    assert!(!crate::routing::filters::provider_accepts_endpoint(
        &channel,
        "/v1/chat/completions"
    ));
    assert!(crate::routing::filters::provider_accepts_endpoint(
        &channel,
        "/v1/responses"
    ));
    let preferences = serde_json::from_value(json!({"protocol_mode":"native"})).unwrap();
    let channel = provider_with_preferences("claude", "https://upstream.example/v1", preferences);
    assert!(!crate::routing::filters::provider_accepts_endpoint(
        &channel,
        "/v1/responses"
    ));
}

#[test]
fn defaults_keep_zero_false_empty_and_null_and_only_add_missing_leaves() {
    let p = provider(
        "gpt",
        "https://upstream.example/v1/chat/completions",
        Some(
            json!({"defaults":{"temperature":0.7,"zero":8,"flag":true,"text":"fallback","nothing":"fallback","nested":{"present":2,"new":3},"blocked":{"new":4}}}),
        ),
    );
    let payload = json!({"model":"public","messages":[],"temperature":0,"zero":0,"flag":false,"text":"","nothing":null,"nested":{"present":null},"blocked":null});
    let result = body(&prepare(&p, "/v1/chat/completions", payload).unwrap());
    assert_eq!(result["temperature"], 0);
    assert_eq!(result["zero"], 0);
    assert_eq!(result["flag"], false);
    assert_eq!(result["text"], "");
    assert!(result["nothing"].is_null());
    assert_eq!(result["nested"], json!({"present":null,"new":3}));
    assert!(result["blocked"].is_null());
}
#[test]
fn deep_merge_array_replace_removal_and_exact_model_order() {
    let p = provider(
        "gpt",
        "https://upstream.example/v1/chat/completions",
        Some(json!({
            "defaults":{"nested":{"default":1}},"channel":{"set":{"nested":{"channel":false,"array":[1,2]},"literal":null,"empty":""},"remove":["/drop","/escaped~1key/~0key"]},
            "models":[{"match":{"requested_model":"wire"},"set":{"wrong":true}},{"match":{"requested_model":"public"},"set":{"nested":{"model":true,"array":[3]}},"remove":["/nested/delete"]},{"match":{"requested_model":"public"},"set":{"last":0}}],"future_extension":{"preserve":[1,2]}
        })),
    );
    let input = json!({"model":"public","messages":[],"nested":{"client":1,"array":[9],"delete":1},"drop":1,"escaped/key":{"~key":1,"kept":2}});
    let prepared = prepare(&p, "/v1/chat/completions", input).unwrap();
    let result = body(&prepared);
    assert_eq!(
        result["nested"],
        json!({"client":1,"default":1,"channel":false,"model":true,"array":[3]})
    );
    assert!(!result.as_object().unwrap().contains_key("drop"));
    assert_eq!(result["escaped/key"], json!({"kept":2}));
    assert!(result.get("wrong").is_none());
    assert_eq!(result["last"], 0);
    assert!(result["literal"].is_null());
    assert_eq!(result["empty"], "");
    assert!(!prepared.policy_report.warnings.is_empty());
    assert_eq!(
        p.preferences["parameter_policy"]["future_extension"],
        json!({"preserve":[1,2]})
    );
}
#[test]
fn adapter_generated_claude_and_gemini_budgets_are_not_client_presence() {
    for (engine, url, path, default) in [
        (
            "claude",
            "https://upstream.example/v1/messages",
            "/max_tokens",
            json!({"max_tokens":512}),
        ),
        (
            "gemini",
            "https://upstream.example/v1beta",
            "/generationConfig/maxOutputTokens",
            json!({"generationConfig":{"maxOutputTokens":512}}),
        ),
    ] {
        let p = provider(engine, url, Some(json!({"defaults":default})));
        assert_eq!(
            body(&prepare(&p, "/v1/chat/completions", chat()).unwrap()).pointer(path),
            Some(&json!(512))
        );
    }
}
#[test]
fn default_budget_alias_never_overrides_a_client_budget() {
    let p = provider(
        "gpt",
        "https://upstream.example/v1/chat/completions",
        Some(json!({"defaults":{"max_completion_tokens":300}})),
    );
    let mut input = chat();
    input["max_tokens"] = json!(80);
    let output = body(&prepare(&p, "/v1/chat/completions", input).unwrap());
    assert_eq!(output["max_tokens"], 80);
    assert!(output.get("max_completion_tokens").is_none());
}
#[test]
fn unrelated_policy_preserves_chat_max_completion_tokens_spelling() {
    let p = provider(
        "gpt",
        "https://upstream.example/v1/chat/completions",
        Some(json!({"channel":{"set":{"user":"audit"}}})),
    );
    let mut input = chat();
    input["max_completion_tokens"] = json!(64);
    let output = body(&prepare(&p, "/v1/chat/completions", input).unwrap());
    assert_eq!(output["max_completion_tokens"], 64);
    assert!(output.get("max_tokens").is_none());
}
#[test]
fn explicit_budget_changes_have_one_wire_field_and_azure_final_mapping() {
    let p = provider(
        "gpt",
        "https://upstream.example/v1/chat/completions",
        Some(json!({"channel":{"set":{"max_completion_tokens":128}}})),
    );
    let mut input = chat();
    input["max_tokens"] = json!(64);
    let output = body(&prepare(&p, "/v1/chat/completions", input).unwrap());
    assert_eq!(output["max_completion_tokens"], 128);
    assert!(output.get("max_tokens").is_none());
    assert!(output.get("max_output_tokens").is_none());
    let p = provider(
        "azure",
        "https://fixture.openai.azure.com/",
        Some(json!({"channel":{"set":{"max_tokens":128}}})),
    );
    let input = PreparedInput {
        payload: Some(chat()),
        replay: None,
        observation: Default::default(),
        default_model: String::new(),
        content_type: "application/json".into(),
    };
    let prepared = crate::plugins::builtin::build_attempt(
        &p,
        "key",
        "public",
        "gpt-5-fixture",
        &Method::POST,
        &"/v1/chat/completions".parse().unwrap(),
        "/v1/chat/completions",
        &HeaderMap::new(),
        &input,
        "test",
    )
    .unwrap();
    let output = body(&prepared);
    assert_eq!(output["max_completion_tokens"], 128);
    assert!(output.get("max_tokens").is_none());
}
#[test]
fn legacy_removal_is_not_resurrected_by_a_default_null_placeholder() {
    let prefs=serde_json::from_value(json!({"post_body_parameter_overrides":{"__remove__":["temperature","nested"]},"parameter_policy":{"defaults":{"temperature":0.4,"nested":{"provided":2}}}})).unwrap();
    let p = provider_with_preferences("gpt", "https://upstream.example/v1/chat/completions", prefs);
    let mut input = chat();
    input["temperature"] = json!(0.7);
    input["nested"] = json!({"provided":1});
    let output = body(&prepare(&p, "/v1/chat/completions", input).unwrap());
    assert!(output.get("temperature").is_none());
    assert!(output.get("nested").is_none());
}
#[test]
fn removing_required_claude_budget_is_an_explicit_error() {
    let p = provider(
        "claude",
        "https://upstream.example/v1/messages",
        Some(json!({"channel":{"remove":["/max_tokens"]}})),
    );
    assert!(prepare(&p, "/v1/chat/completions", chat())
        .unwrap_err_for_policy()
        .contains("required"));
}
// PreparedAttempt intentionally has no Debug (contains credentials). Avoid
// unwrap_err's Debug bound without exposing it through test failure messages.
trait PolicyError<T> {
    fn unwrap_err_for_policy(self) -> String;
}
impl<T> PolicyError<T> for Result<T, String> {
    fn unwrap_err_for_policy(self) -> String {
        match self {
            Err(error) => error,
            Ok(_) => panic!("expected a policy error"),
        }
    }
}
#[test]
fn invalid_policy_and_protocol_constraints_are_errors_without_values() {
    for policy in [
        json!({"headers":[{"name":"x-demo","mode":"set"}]}),
        json!({"headers":[{"name":"x-demo","mode":"set","value":"secret\r\nvalue"}]}),
        json!({"headers":[{"name":"Host","mode":"remove"}]}),
        json!({"channel":{"remove":["not-a-pointer"]}}),
        json!({"models":[{"match":{"upstream_model":"wrong"}}]}),
        json!({"channel":{"set":{"stream":false}}}),
    ] {
        let error = validate_policy(&policy).unwrap_err();
        assert!(!error.contains("secret"));
    }
    for policy in [
        json!({"channel":{"set":{"temperature":3}}}),
        json!({"channel":{"set":{"max_tokens":0}}}),
        json!({"channel":{"set":{"max_tokens":10,"max_completion_tokens":20}}}),
        json!({"channel":{"set":{"service_tier":"anything"}}}),
    ] {
        let p = provider(
            "gpt",
            "https://upstream.example/v1/chat/completions",
            Some(policy),
        );
        assert!(prepare(&p, "/v1/chat/completions", chat()).is_err());
    }
    let p = provider(
        "gpt",
        "https://upstream.example/v1/responses",
        Some(json!({"channel":{"set":{"max_tokens":42}}})),
    );
    let reg = crate::plugins::PluginRegistry::open(
        None,
        TrustPolicy {
            allowed_origins: vec![],
            allow_loopback_http: false,
        },
    )
    .unwrap();
    assert!(prepare_preview(
        &reg,
        &p,
        "public",
        "wire",
        "/v1/responses",
        json!({"input":"hello"})
    )
    .is_err());
}
#[test]
fn chat_prompt_modes_preserve_developer_and_ordinary_order() {
    for mode in ["prepend", "append", "replace", "remove", "inherit"] {
        let p = provider(
            "gpt",
            "https://upstream.example/v1/chat/completions",
            Some(json!({"system_prompt":{"mode":mode,"value":"new"}})),
        );
        let output = body(&prepare(&p, "/v1/chat/completions", chat()).unwrap());
        let messages = output["messages"].as_array().unwrap();
        let ordinary = messages
            .iter()
            .filter(|m| m["role"] != "system")
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            ordinary,
            vec![
                json!({"role":"developer","content":"developer must stay"}),
                json!({"role":"user","content":"hello"})
            ]
        );
        let system = messages
            .iter()
            .filter(|m| m["role"] == "system")
            .map(|m| m["content"].clone())
            .collect::<Vec<_>>();
        let expected = match mode {
            "prepend" => vec![json!("new"), json!("old")],
            "append" => vec![json!("old"), json!("new")],
            "replace" => vec![json!("new")],
            "remove" => vec![],
            _ => vec![json!("old")],
        };
        assert_eq!(system, expected);
    }
}
#[test]
fn responses_claude_gemini_prompt_modes_and_model_priority() {
    for mode in ["prepend", "append", "replace", "remove"] {
        for (engine, url, path) in [
            ("claude", "https://upstream.example/v1/messages", "/system"),
            (
                "gemini",
                "https://upstream.example/v1beta",
                "/systemInstruction",
            ),
        ] {
            let p = provider(
                engine,
                url,
                Some(json!({"system_prompt":{"mode":mode,"value":"new"}})),
            );
            let output = body(&prepare(&p, "/v1/chat/completions", chat()).unwrap());
            if mode == "remove" {
                assert!(output.pointer(path).is_none());
            } else {
                assert!(output.pointer(path).unwrap().to_string().contains("new"));
                if mode != "replace" {
                    assert!(output.pointer(path).unwrap().to_string().contains("old"));
                }
            }
        }
        let p = provider(
            "gpt",
            "https://upstream.example/v1/responses",
            Some(json!({"system_prompt":{"mode":mode,"value":"new"}})),
        );
        let mut body = json!({"model":"public","input":"hello","instructions":"old"});
        crate::upstream::responses::route::compile_payload(
            &mut body, &p, "public", "wire", "gpt", false,
        )
        .unwrap();
        let expected = match mode {
            "prepend" => Some("new\nold"),
            "append" => Some("old\nnew"),
            "replace" => Some("new"),
            _ => None,
        };
        assert_eq!(body.get("instructions").and_then(Value::as_str), expected);
    }
    let p = provider(
        "gpt",
        "https://upstream.example/v1/responses",
        Some(
            json!({"system_prompt":{"mode":"replace","value":"channel"},"models":[{"match":{"requested_model":"public"},"set":{"instructions":"model wins"}}]}),
        ),
    );
    let mut output = json!({"input":"hello","instructions":"client"});
    crate::upstream::responses::route::compile_payload(
        &mut output,
        &p,
        "public",
        "wire",
        "gpt",
        false,
    )
    .unwrap();
    assert_eq!(output["instructions"], "model wins");
}
#[test]
fn empty_prompt_replace_is_a_value_and_not_remove() {
    let p = provider(
        "gpt",
        "https://upstream.example/v1/chat/completions",
        Some(json!({"system_prompt":{"mode":"replace","value":""}})),
    );
    let output = body(&prepare(&p, "/v1/chat/completions", chat()).unwrap());
    assert_eq!(output["messages"][0]["content"], "");
    assert_eq!(output["messages"][0]["role"], "system");
}
#[test]
fn header_rules_are_ordered_empty_is_legal_and_report_contains_no_secrets() {
    let p = provider(
        "gpt",
        "https://upstream.example/v1/chat/completions",
        Some(
            json!({"headers":[{"name":"Authorization","mode":"set","value":"Bearer hidden-credential","secret":false},{"name":"x-demo","mode":"set","value":""},{"name":"x-disabled","mode":"set","enabled":false},{"name":"x-remove","mode":"remove"},{"name":"x-private","mode":"set","value":"opaque-hidden","secret":true}]}),
        ),
    );
    let prepared = prepare(&p, "/v1/chat/completions", chat()).unwrap();
    assert_eq!(
        prepared.headers["authorization"],
        "Bearer hidden-credential"
    );
    assert_eq!(prepared.headers["x-demo"], "");
    assert!(!prepared.headers.contains_key("x-disabled"));
    let trace = serde_json::to_string(&prepared.policy_report).unwrap();
    assert!(!trace.contains("hidden-credential"));
    assert!(!trace.contains("opaque-hidden"));
    assert!(prepared
        .policy_report
        .sensitive_headers
        .contains(&"authorization".into()));
    assert!(prepared
        .policy_report
        .sensitive_headers
        .contains(&"x-private".into()));
    assert!(header_sensitive("Cookie", false));
    assert!(header_sensitive("x-api-key", false));
    assert!(!header_sensitive("x-demo", false));
}
#[test]
fn non_conversation_body_policy_is_explicitly_skipped() {
    let p = provider(
        "gpt",
        "https://upstream.example/v1",
        Some(
            json!({"defaults":{"temperature":0.1},"system_prompt":{"mode":"replace","value":"not for embeddings"}}),
        ),
    );
    let prepared = prepare(
        &p,
        "/v1/embeddings",
        json!({"model":"public","input":"hello"}),
    )
    .unwrap();
    let output = body(&prepared);
    assert!(output.get("temperature").is_none());
    assert!(output.get("messages").is_none());
    assert!(!prepared.policy_report.body_supported);
    assert!(prepared
        .policy_report
        .trace
        .iter()
        .any(|t| t.action == "skipped"));
}
#[test]
fn no_policy_keeps_legacy_overrides_and_codex_constraints_unchanged() {
    let prefs=serde_json::from_value(json!({"post_body_parameter_overrides":{"temperature":0.5,"store":true,"response_format":{"type":"json_object"},"public":{"user":"legacy"}}})).unwrap();
    let p = provider_with_preferences("codex", "https://upstream.example/v1/responses", prefs);
    let prepared = prepare(&p, "/v1/chat/completions", chat()).unwrap();
    let output = body(&prepared);
    assert!(!prepared.policy_report.configured);
    assert_eq!(output["store"], false);
    assert!(output.get("temperature").is_none());
    assert!(output.get("response_format").is_none());
    assert_eq!(output["user"], "legacy");
}
#[test]
fn final_codex_constraints_run_after_new_policy_and_are_reported() {
    let p = provider(
        "codex",
        "https://upstream.example/v1/responses",
        Some(json!({"channel":{"set":{"store":true,"response_format":{"type":"json_object"}}}})),
    );
    let prepared = prepare(&p, "/v1/chat/completions", chat()).unwrap();
    let output = body(&prepared);
    assert_eq!(output["store"], false);
    assert!(output.get("response_format").is_none());
    assert!(prepared
        .policy_report
        .trace
        .iter()
        .any(|t| t.path == "/store" && t.source == "protocol_constraint"));
    let mut output = json!({"model":"public","input":"hello"});
    let report = crate::upstream::responses::route::compile_payload(
        &mut output,
        &p,
        "public",
        "wire",
        "codex",
        true,
    )
    .unwrap();
    assert!(output.get("store").is_none());
    assert!(report
        .trace
        .iter()
        .any(|t| t.source == "protocol_constraint"));
}
#[test]
fn preview_uses_native_production_payload_and_shared_selection_predicate() {
    let p = provider(
        "gpt",
        "https://upstream.example/prefix/v1/responses",
        Some(
            json!({"defaults":{"max_output_tokens":512},"channel":{"set":{"temperature":0.4}},"system_prompt":{"mode":"prepend","value":"prefix"},"headers":[{"name":"x-policy","mode":"set","value":"applied"}]}),
        ),
    );
    let reg = crate::plugins::PluginRegistry::open(
        None,
        TrustPolicy {
            allowed_origins: vec![],
            allow_loopback_http: false,
        },
    )
    .unwrap();
    let input = json!({"model":"public","input":"hello","instructions":"old","temperature":0.8});
    let preview =
        prepare_preview(&reg, &p, "public", "wire", "/v1/responses", input.clone()).unwrap();
    let mut real = input;
    crate::upstream::responses::route::compile_payload(
        &mut real, &p, "public", "wire", "gpt", false,
    )
    .unwrap();
    assert_eq!(body(&preview), real);
    assert_eq!(preview.headers["x-policy"], "applied");
    let p = provider("gpt", "https://upstream.example/foo/responses", None);
    assert!(!native_responses_candidate(&p, "/v1/responses"));
    assert_eq!(capabilities(&p, "/v1/responses")["profile"], "responses");
}
#[test]
fn service_policy_changes_only_envelope_request_body_and_rejects_control_headers() {
    use crate::plugins::registry::Registration;
    let reg = crate::plugins::PluginRegistry::open(
        None,
        TrustPolicy {
            allowed_origins: vec![],
            allow_loopback_http: true,
        },
    )
    .unwrap();
    let registration:Registration=serde_json::from_value(json!({"manifest":{"id":"policy-service","name":"Fixture","version":"1","api_version":1,"capabilities":{"endpoints":["/v1/chat/completions"],"streaming":true,"model_discovery":false}},"endpoint":"http://127.0.0.1:8765","service_auth":{"type":"bearer","token":"control-secret"},"expected_revision":0})).unwrap();
    reg.register(registration).unwrap();
    let p = provider(
        "plugin:policy-service",
        "",
        Some(json!({"defaults":{"max_tokens":512},"channel":{"set":{"temperature":0.3}}})),
    );
    let attempt =
        prepare_preview(&reg, &p, "public", "wire", "/v1/chat/completions", chat()).unwrap();
    let output = body(&attempt);
    assert_eq!(output["request"]["body"]["max_tokens"], 512);
    assert_eq!(output["request"]["body"]["temperature"], 0.3);
    assert!(output.get("temperature").is_none());
    assert_eq!(attempt.headers["authorization"], "Bearer control-secret");
    assert_eq!(attempt.policy_report.scope, "service_request_body");
    let p = provider(
        "plugin:policy-service",
        "",
        Some(json!({"headers":[{"name":"authorization","mode":"set","value":"replacement"}]})),
    );
    assert!(
        prepare_preview(&reg, &p, "public", "wire", "/v1/chat/completions", chat())
            .unwrap_err_for_policy()
            .contains("control-channel")
    );
}

#[test]
fn duplicate_header_names_are_rejected_even_when_disabled() {
    for disabled in [false, true] {
        let policy = json!({"headers":[{"name":"X-Secret","mode":"set","value":"one","secret":true},{"name":"x-secret","mode":"inherit","enabled":!disabled}]});
        assert!(validate_policy(&policy).unwrap_err().contains("duplicate"));
    }
}
#[test]
fn uppercase_codex_preview_follows_generic_production_and_native_stream_coercion_matches() {
    let registry = crate::plugins::PluginRegistry::open(
        None,
        TrustPolicy {
            allowed_origins: vec![],
            allow_loopback_http: false,
        },
    )
    .unwrap();
    let p = provider(
        "CODEX",
        "https://upstream.example/v1/responses",
        Some(json!({"channel":{"set":{"store":true}}})),
    );
    assert!(!native_responses_candidate(&p, "/v1/responses"));
    let input = json!({"model":"public","input":"hello","previous_response_id":"existing","max_output_tokens":512});
    let preview = prepare_preview(
        &registry,
        &p,
        "public",
        "wire",
        "/v1/responses",
        input.clone(),
    )
    .unwrap();
    let actual = prepare(&p, "/v1/responses", input).unwrap();
    assert_eq!(body(&preview), body(&actual));
    assert_eq!(preview.url, actual.url);
    let p = provider("gpt", "https://upstream.example/v1/responses", None);
    let preview = prepare_preview(
        &registry,
        &p,
        "public",
        "wire",
        "/v1/responses",
        json!({"model":"public","input":"hello","stream":"false"}),
    )
    .unwrap();
    assert_eq!(body(&preview)["stream"], false);
}
