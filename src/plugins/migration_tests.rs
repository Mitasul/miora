use super::tests::{registration, registry};
use axum::http::{HeaderMap, Method};
use serde_json::{json, Map, Value};
use std::sync::{atomic::AtomicUsize, Arc};

#[test]
fn migration_compat_example_manifest_installs_without_channel_credentials() {
    let manifest: super::manifest::Manifest = serde_json::from_str(include_str!(
        "../../examples/plugins/managed-openai/manifest.json"
    ))
    .unwrap();
    manifest.validate().unwrap();
    assert!(manifest.config(&Map::new(), false).is_ok());
    assert!(manifest.config(&Map::new(), true).is_err());
    let supplied = Map::from_iter([("api_key".into(), json!("key"))]);
    assert_eq!(
        manifest.config(&supplied, true).unwrap()["base_url"],
        "https://api.openai.com/v1"
    );
}

#[test]
fn migration_compat_multimodal_manifest_is_opt_in_and_legacy_json_is_unchanged() {
    let manifest: super::manifest::Manifest = serde_json::from_str(include_str!(
        "../../examples/plugins/managed-multimodal/manifest.json"
    ))
    .unwrap();
    manifest.validate().unwrap();
    assert!(manifest.config(&Map::new(), false).is_ok());
    assert!(manifest.config(&Map::new(), true).is_err());
    assert_eq!(
        manifest.capabilities.request_formats,
        ["json", "form", "multipart", "binary"]
    );
    assert_eq!(
        registration(0, "1.0.0")
            .manifest
            .capabilities
            .request_formats,
        ["json"]
    );
}

#[test]
fn migration_compat_unsigned_thinking_does_not_leave_empty_assistant_messages() {
    let mut payload = json!({"messages":[
        {"role":"user","content":"question"},
        {"role":"assistant","content":[{"type":"thinking","thinking":"unsigned"}]},
        {"role":"assistant","content":[]},
        {"role":"assistant","content":[{"type":"thinking","thinking":"signed","signature":"valid"}]},
        {"role":"assistant","content":[{"type":"thinking","thinking":"unsigned"},{"type":"tool_use","id":"tool-1","name":"search","input":{}}]}
    ]});
    crate::providers::anthropic::strip_unsigned_thinking(&mut payload);
    let messages = payload["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 4);
    assert_eq!(messages[1]["content"], json!([]));
    assert_eq!(messages[2]["content"][0]["signature"], "valid");
    assert_eq!(messages[3]["content"][0]["id"], "tool-1");
}

#[test]
fn migration_compat_unsigned_thinking_only_removes_newly_empty_assistant_content() {
    let unsigned = json!({"type":"thinking","thinking":"unsigned"});
    let empty_text = json!({"type":"text","text":""});
    let whitespace_text = json!({"type":"text","text":" \t\r\n"});
    let text = json!({"type":"text","text":"answer"});
    let signed = json!({"type":"thinking","thinking":"signed","signature":"opaque-signature"});
    let redacted = json!({"type":"redacted_thinking","data":"opaque"});
    let tool = json!({"type":"tool_use","id":"tool-1","name":"search","input":{}});
    for (content, expected) in [
        (json!([unsigned]), None),
        (json!([unsigned, empty_text]), None),
        (json!([unsigned, whitespace_text, empty_text]), None),
        (
            json!([{"type":"thinking","signature":null}, empty_text]),
            None,
        ),
        (
            json!([{"type":"thinking","signature":42}, empty_text]),
            None,
        ),
        (
            json!([{"type":"thinking","signature":""}, empty_text]),
            None,
        ),
        (
            json!([{"type":"thinking","signature":" \t"}, empty_text]),
            None,
        ),
        (json!([unsigned, text]), Some(json!([text]))),
        (
            json!([unsigned, signed, empty_text]),
            Some(json!([signed, empty_text])),
        ),
        (
            json!([unsigned, redacted, empty_text]),
            Some(json!([redacted, empty_text])),
        ),
        (
            json!([unsigned, tool, empty_text]),
            Some(json!([tool, empty_text])),
        ),
        (
            json!([unsigned, {"type":"vendor-extension","data":{}}]),
            Some(json!([{"type":"vendor-extension","data":{}}])),
        ),
        (
            json!([unsigned, {"type":"text","text":null}]),
            Some(json!([{"type":"text","text":null}])),
        ),
        (json!([]), Some(json!([]))),
        (json!([empty_text]), Some(json!([empty_text]))),
        (json!([whitespace_text]), Some(json!([whitespace_text]))),
        (json!(""), Some(json!(""))),
        (Value::Null, Some(Value::Null)),
    ] {
        let mut payload = json!({"messages":[
            {"role":"user","content":content},
            {"role":"assistant","content":content}
        ]});
        crate::providers::anthropic::strip_unsigned_thinking(&mut payload);
        let messages = payload["messages"].as_array().unwrap();
        assert_eq!(messages[0], json!({"role":"user","content":content}));
        match expected {
            Some(expected) => {
                assert_eq!(messages.len(), 2);
                assert_eq!(messages[1]["content"], expected);
            }
            None => assert_eq!(messages.len(), 1, "content: {content}"),
        }
        let first = payload.clone();
        crate::providers::anthropic::strip_unsigned_thinking(&mut payload);
        assert_eq!(payload, first);
    }
}

#[test]
fn migration_compat_claude_empty_assistant_remnants_cover_native_and_converted_paths() {
    let signed = json!({"type":"thinking","thinking":"signed","signature":"opaque-signature"});
    let blank = json!({"type":"text","text":" \t"});
    let payload = json!({"model":"public","messages":[
        {"role":"user","content":"first"},
        {"role":"assistant","content":[{"type":"thinking","thinking":"unsigned"},blank]},
        {"role":"user","content":"second"},
        {"role":"assistant","content":[{"type":"thinking","thinking":"unsigned"},{"type":"text","text":"answer"}]},
        {"role":"user","content":"third"},
        {"role":"assistant","content":[signed,blank]},
        {"role":"user","content":"continue"}
    ]});
    for path in ["/v1/messages", "/v1/chat/completions"] {
        let preserve = body(prepare(
            &provider("claude", json!({})),
            payload.clone(),
            path,
            &HeaderMap::new(),
            "preserve-empty",
        ));
        assert_eq!(preserve["messages"].as_array().unwrap().len(), 7);
        assert_eq!(
            preserve["messages"][1]["content"],
            payload["messages"][1]["content"]
        );
        let stripped = body(prepare(
            &provider("claude", json!({"unsigned_thinking":"strip"})),
            payload.clone(),
            path,
            &HeaderMap::new(),
            "strip-empty",
        ));
        let messages = stripped["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 6, "path: {path}");
        assert_eq!(
            messages[2]["content"],
            json!([{"type":"text","text":"answer"}])
        );
        assert_eq!(messages[4]["content"], json!([signed, blank]));
        assert_eq!(messages[5]["role"], "user");
    }
}

#[test]
fn migration_compat_plugin_private_fields_bypass_generic_parameter_removal() {
    let reg = registry();
    let mut registration = registration(0, "1.0.0");
    registration.manifest.request_schema = serde_json::from_value(json!([
        {"key":"vendorStyle","type":"string","enum":["ink","photo"]},
        {"key":"temperature","type":"string"}
    ]))
    .unwrap();
    reg.register(registration).unwrap();
    let provider = provider(
        "plugin:echo",
        json!({"parameter_policy":{"channel":{"remove":["/vendorStyle","/temperature"]}}}),
    );
    let input = crate::providers::types::PreparedInput {
        payload: Some(
            json!({"model":"public","messages":[{"role":"user","content":"draw"}],"vendorStyle":"ink","temperature":"vendor-specific","otherExtension":{"keep":true}}),
        ),
        replay: None,
        observation: Default::default(),
        default_model: String::new(),
        content_type: "application/json".into(),
    };
    let attempt = reg
        .prepare(
            &provider,
            "key",
            "public",
            "upstream",
            &Method::POST,
            &"/v1/chat/completions?vendorStyle=photo&key=gateway-secret"
                .parse()
                .unwrap(),
            "/v1/chat/completions",
            &HeaderMap::new(),
            &input,
            "private-fields",
        )
        .unwrap();
    assert!(attempt
        .policy_report
        .trace
        .iter()
        .any(|item| item.path == "/vendorStyle" && item.source == "plugin_schema"));
    let wire = body(attempt);
    assert_eq!(
        wire["request"]["query_parameters"],
        json!({"vendorStyle":"photo"})
    );
    assert_eq!(wire["request"]["body"]["vendorStyle"], "ink");
    assert_eq!(wire["request"]["body"]["temperature"], "vendor-specific");
    assert_eq!(
        wire["request"]["body"]["otherExtension"],
        json!({"keep":true})
    );
    assert!(wire["request"].get("body_resource").is_none());
}

#[tokio::test]
async fn migration_compat_plugin_non_json_input_uses_scoped_resource_transport() {
    let reg = registry();
    let mut registration = registration(0, "1.0.0");
    registration
        .manifest
        .capabilities
        .endpoints
        .push("/v1/images/edits".into());
    registration.manifest.capabilities.request_formats = vec![
        "json".into(),
        "form".into(),
        "multipart".into(),
        "binary".into(),
    ];
    reg.register(registration).unwrap();
    let provider = provider("plugin:echo", json!({}));
    let manager = crate::transport::spool::SpoolManager::new(
        crate::runtime::resources::ResourceGovernor::unconstrained_for_test(),
    )
    .unwrap();
    for (content_type, format, bytes) in [
        ("application/x-www-form-urlencoded", "form", &b"model=public&seed=42"[..]),
        ("multipart/form-data; boundary=fixture", "multipart", &b"--fixture\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\npublic\r\n--fixture--\r\n"[..]),
        ("image/png", "binary", &b"\x89PNG\x00\xff"[..]),
    ] {
        let mut writer = manager.begin(None, Some(bytes.len() as u64), std::time::Duration::ZERO).await.unwrap();
        writer.append(bytes::Bytes::copy_from_slice(bytes)).await.unwrap();
        let spool = writer.finish().await.unwrap();
        if format == "form" {
            assert_eq!(spool.storage.urlencoded_routing_fields().await.unwrap()["model"], "public");
        }
        if format == "multipart" {
            assert_eq!(spool.storage.multipart_text_fields(content_type, &["model", "stream"], 4096).await.unwrap()["model"], "public");
        }
        let input = crate::providers::types::PreparedInput { payload:Some(json!({"model":"public"})), replay:Some((spool.storage, spool.observation.clone())), observation:spool.observation, default_model:String::new(), content_type:content_type.into() };
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer gateway-secret".parse().unwrap());
        let attempt = reg.prepare(&provider, "key", "public", "upstream", &Method::POST, &"/v1/images/edits".parse().unwrap(), "/v1/images/edits", &headers, &input, "resource").unwrap();
        assert_eq!(attempt.headers["content-type"], "application/vnd.miora.resource+octet-stream");
        let crate::providers::types::AttemptBody::PluginResource { envelope, storage, observation } = attempt.body else { panic!("resource body required") };
        let wire: Value = serde_json::from_slice(&envelope).unwrap();
        assert!(!String::from_utf8(envelope.clone()).unwrap().contains("gateway-secret"));
        assert_eq!(wire["request"]["format"], format);
        assert_eq!(wire["request"]["body_resource"]["content_type"], content_type);
        assert_eq!(wire["request"]["body_resource"]["size"], bytes.len());
        assert_eq!(attempt.headers["x-miora-envelope-length"].to_str().unwrap().parse::<usize>().unwrap(), envelope.len());
        assert_eq!(attempt.headers["content-length"].to_str().unwrap().parse::<usize>().unwrap(), envelope.len() + bytes.len());
        let replay = axum::body::to_bytes(storage.into_body(&observation).await.unwrap(), 4096).await.unwrap();
        assert_eq!(replay.as_ref(), bytes);
    }
}

fn provider(engine: &str, preferences: Value) -> Arc<crate::config::snapshot::Provider> {
    let raw = serde_json::from_value(json!({"name":"migration","base_url":"https://example.invalid/v1","engine":engine,"api":"upstream","models":{"public":"upstream-model"},"preferences":preferences})).unwrap();
    crate::config::snapshot::runtime_provider(raw, Arc::new(AtomicUsize::new(0)))
}

fn prepare(
    provider: &crate::config::snapshot::Provider,
    payload: Value,
    path: &str,
    headers: &HeaderMap,
    request_id: &str,
) -> crate::providers::types::PreparedAttempt {
    let input = crate::providers::types::PreparedInput {
        payload: Some(payload),
        replay: None,
        observation: Default::default(),
        default_model: String::new(),
        content_type: "application/json".into(),
    };
    super::builtin::build_attempt(
        provider,
        "upstream-key",
        "public",
        "upstream-model",
        &Method::POST,
        &path.parse().unwrap(),
        path,
        headers,
        &input,
        request_id,
    )
    .unwrap()
}

fn body(attempt: crate::providers::types::PreparedAttempt) -> Value {
    let crate::providers::types::AttemptBody::Json(bytes) = attempt.body else {
        panic!("expected JSON");
    };
    serde_json::from_slice(&bytes).unwrap()
}

#[test]
fn migration_compat_codex_chat_wire_preserves_cache_metadata_and_session_precedence() {
    let provider = provider("codex", json!({}));
    let payload = json!({"model":"public","messages":[{"role":"user","content":"fixed prefix"}],"stream":true,"prompt_cache_key":"cache/prefix","client_metadata":{"session":"original","zero":0}});
    let mut headers = HeaderMap::new();
    headers.insert("Session-Id", "hyphen-session".parse().unwrap());
    headers.insert("session_id", "underscore-session".parse().unwrap());
    let first = prepare(
        &provider,
        payload.clone(),
        "/v1/chat/completions",
        &headers,
        "request-one",
    );
    assert_eq!(first.headers["session_id"], "underscore-session");
    assert_eq!(first.headers["authorization"], "Bearer upstream-key");
    let wire = body(first);
    assert_eq!(wire["prompt_cache_key"], payload["prompt_cache_key"]);
    assert_eq!(wire["client_metadata"], payload["client_metadata"]);
    assert_eq!(wire["stream"], true);
    assert_eq!(wire["model"], "upstream-model");
    let first = prepare(
        &provider,
        payload.clone(),
        "/v1/chat/completions",
        &HeaderMap::new(),
        "request-one",
    );
    let second = prepare(
        &provider,
        payload,
        "/v1/chat/completions",
        &HeaderMap::new(),
        "request-two",
    );
    assert_eq!(first.headers["session_id"], second.headers["session_id"]);
    assert_eq!(first.headers["session_id"], "cache%2Fprefix");
}

#[test]
fn migration_compat_codex_search_summary_and_compact_use_distinct_rules() {
    let provider = provider("codex", json!({}));
    let payload = json!({"model":"public","input":[{"type":"web_search_call","id":"history"}],"tools":[],"tool_choice":"auto","instructions":"Summarize the conversation"});
    let wire = body(prepare(
        &provider,
        payload.clone(),
        "/v1/responses",
        &HeaderMap::new(),
        "one",
    ));
    assert_eq!(wire["tools"], json!([{"type":"web_search"}]));
    assert_eq!(wire["tool_choice"], "none");
    assert_eq!(wire["input"][0]["id"], "history");
    let compact = body(prepare(
        &provider,
        payload,
        "/v1/responses/compact",
        &HeaderMap::new(),
        "two",
    ));
    assert_eq!(compact["tools"], json!([]));
    assert_eq!(compact["tool_choice"], "auto");
}

#[test]
fn migration_compat_claude_policy_preserves_signed_blocks_and_is_channel_scoped() {
    let content = json!([{"type":"thinking","thinking":"unsigned"},{"type":"thinking","thinking":"blank","signature":" "},{"type":"thinking","thinking":"signed","signature":"opaque-signature"},{"type":"redacted_thinking","data":"opaque"},{"type":"text","text":"answer"}]);
    for path in ["/v1/messages", "/v1/chat/completions"] {
        let payload = json!({"model":"public","messages":[{"role":"assistant","content":content},{"role":"user","content":"continue"}]});
        let preserve = body(prepare(
            &provider("claude", json!({})),
            payload.clone(),
            path,
            &HeaderMap::new(),
            "one",
        ));
        assert_eq!(preserve["messages"][0]["content"], content);
        let stripped = body(prepare(
            &provider("claude", json!({"unsigned_thinking":"strip"})),
            payload,
            path,
            &HeaderMap::new(),
            "two",
        ));
        assert_eq!(
            stripped["messages"][0]["content"],
            json!([content[2], content[3], content[4]])
        );
    }
    let translated = crate::protocols::dialects::messages_to_chat_with_thinking(
        &json!({"messages":[{"role":"assistant","content":content}]}),
        "model",
        true,
    )
    .unwrap();
    let translated = crate::providers::anthropic::chat_to_claude(&translated, "model").unwrap();
    assert_eq!(translated["messages"][0]["content"], content);
}

#[test]
fn migration_compat_codex_image_inputs_keep_data_urls_and_detail() {
    let provider = provider("codex", json!({}));
    let image = "data:image/png;base64,fixture";
    let chat = json!({"messages":[{"role":"user","content":[{"type":"text","text":"describe"},{"type":"image_url","image_url":{"url":image,"detail":"high"}}]}]});
    let wire = body(prepare(
        &provider,
        chat,
        "/v1/chat/completions",
        &HeaderMap::new(),
        "one",
    ));
    assert_eq!(
        wire["input"][0]["content"][1],
        json!({"type":"input_image","image_url":image,"detail":"high"})
    );
    let native = json!({"input":[{"role":"user","content":[{"type":"input_image","image_url":image,"detail":"low"}]}],"tools":[{"type":"image_generation"}]});
    let wire = body(prepare(
        &provider,
        native.clone(),
        "/v1/responses",
        &HeaderMap::new(),
        "two",
    ));
    assert_eq!(wire["input"], native["input"]);
    assert_eq!(wire["tools"], native["tools"]);
    let chat = crate::protocols::conversion::responses_input_to_chat_content(Some(
        &native["input"][0]["content"],
    ));
    assert_eq!(chat[0]["image_url"]["detail"], "low");
}

#[test]
fn migration_compat_dashscope_compatible_chat_preserves_stream_tools_and_mapping() {
    let mut provider = (*provider("gpt", json!({}))).clone();
    provider.base_url = "https://dashscope.aliyuncs.com/compatible-mode/v1".into();
    let payload = json!({"model":"public","messages":[{"role":"user","content":"hi"}],"stream":true,"stream_options":{"include_usage":true},"tools":[{"type":"function","function":{"name":"lookup","parameters":{"type":"object"}}}],"tool_choice":"auto"});
    let attempt = prepare(
        &provider,
        payload.clone(),
        "/v1/chat/completions",
        &HeaderMap::new(),
        "one",
    );
    assert_eq!(
        attempt.url,
        "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions"
    );
    assert_eq!(attempt.headers["authorization"], "Bearer upstream-key");
    let wire = body(attempt);
    assert_eq!(wire["tools"], payload["tools"]);
    assert_eq!(wire["stream_options"], payload["stream_options"]);
    assert_eq!(wire["model"], "upstream-model");
}

#[test]
fn migration_compat_plugin_update_defaults_and_rollback_config_are_independent() {
    let reg = registry();
    reg.register(registration(0, "1.0.0")).unwrap();
    let provider = provider("plugin:echo", json!({"channel_plugin":"echo"}));
    let mut new = registration(1, "2.0.0");
    new.manifest.config_schema.push(
        serde_json::from_value(json!({"key":"required_field","type":"string","required":true}))
            .unwrap(),
    );
    new.config
        .insert("required_field".into(), json!("new-default"));
    reg.validate_instances(&new.manifest, Some(&new.config), &[provider.clone()])
        .unwrap();
    assert!(reg
        .validate_instances(&new.manifest, None, &[provider.clone()])
        .is_err());
    reg.register(new).unwrap();
    reg.validate_rollback_instances("echo", 1, &[provider])
        .unwrap();
    let mut new = registration(2, "3.0.0");
    new.config = Map::new();
    reg.register(new).unwrap();
    assert_eq!(
        reg.snapshot().versions["echo"].config["credential"],
        "channel-secret"
    );
}

#[test]
fn migration_compat_plugin_configuration_rename_is_projected_without_source_changes() {
    let mut request = registration(0, "1.0.0");
    request.manifest.config_schema[1].key = "token".into();
    request
        .manifest
        .config_aliases
        .insert("token".into(), "credential".into());
    let original = request.config.clone();
    let result = request.manifest.config(&request.config, true).unwrap();
    assert_eq!(result["token"], "channel-secret");
    assert_eq!(request.config, original);
    assert_eq!(request.manifest.redact(&original)["credential"], "***");
}

#[test]
fn migration_compat_plugin_history_retains_three_versions_and_live_leases() {
    let reg = registry();
    reg.register(registration(0, "1.0.0")).unwrap();
    let pinned = reg.pin();
    for revision in 1..7 {
        reg.register(registration(revision, &format!("{}.0.0", revision + 1)))
            .unwrap();
    }
    assert_eq!(pinned.snapshot().versions["echo"].manifest.version, "1.0.0");
    assert!(reg.validate_rollback_instances("echo", 1, &[]).is_ok());
    drop(pinned);
    reg.register(registration(7, "8.0.0")).unwrap();
    assert!(reg.validate_rollback_instances("echo", 1, &[]).is_err());
    assert_eq!(
        reg.list()["plugins"][0]["versions"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}
