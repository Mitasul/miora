use super::*;
use crate::config::snapshot::{runtime_provider, ApiKey, Provider, RawProvider, Snapshot};
use crate::providers::types::{AttemptBody, PreparedInput, ResponseAdapter};
use axum::http::{HeaderMap, Method};
use std::sync::{atomic::AtomicUsize, Arc};

fn provider(engine: &str, preferences: Value) -> Arc<Provider> {
    let source = json!({"provider":"primary","base_url":if engine == "gemini" {"https://example.test/v1beta"} else {"https://example.test/v1"},"engine":engine,"api":["shared-key"],"model":["sample"],"preferences":preferences});
    let raw: RawProvider =
        serde_json::from_value(crate::config::compiler::compile_provider(&source).unwrap())
            .unwrap();
    runtime_provider(raw, Arc::new(AtomicUsize::new(0)))
}

fn attempt(
    provider: &Provider,
    path: &str,
    payload: Value,
) -> crate::providers::types::PreparedAttempt {
    crate::plugins::builtin::build_attempt(
        provider,
        "upstream-key",
        "sample",
        "actual",
        &Method::POST,
        &path.parse().unwrap(),
        path,
        &HeaderMap::new(),
        &PreparedInput {
            payload: Some(payload),
            replay: None,
            observation: Default::default(),
            default_model: String::new(),
            content_type: "application/json".into(),
        },
        "test",
    )
    .unwrap()
}

fn body(attempt: crate::providers::types::PreparedAttempt) -> Value {
    match attempt.body {
        AttemptBody::Json(bytes) => serde_json::from_slice(&bytes).unwrap(),
        _ => panic!("expected JSON"),
    }
}

#[test]
fn gateway_features_gemini_native_preserves_extension_fields_and_uses_sse() {
    let payload = json!({"contents":[{"role":"user","parts":[{"text":"hello","thoughtSignature":"opaque"}]}],"generationConfig":{"temperature":0.3},"vendorExtension":{"keep":true}});
    let prepared = attempt(
        &provider("gemini", json!({})),
        "/v1beta/models/sample:streamGenerateContent",
        payload.clone(),
    );
    assert_eq!(prepared.adapter, ResponseAdapter::Passthrough);
    assert_eq!(prepared.downstream_protocol, DownstreamProtocol::Native);
    assert!(prepared.upstream_stream);
    let url = url::Url::parse(&prepared.url).unwrap();
    assert_eq!(url.path(), "/v1beta/models/actual:streamGenerateContent");
    assert!(url
        .query_pairs()
        .any(|(key, value)| key == "alt" && value == "sse"));
    assert_eq!(body(prepared), payload);
    assert_eq!(gemini_route("/v1beta/models/sample:countTokens"), None);
}

#[test]
fn gateway_features_claude_native_keeps_signed_thinking_and_cache_controls() {
    let payload = json!({"model":"sample","messages":[{"role":"assistant","content":[{"type":"thinking","thinking":"opaque thought","signature":"upstream-signature"},{"type":"text","text":"answer","cache_control":{"type":"ephemeral"}}]}],"max_tokens":42,"context_management":{"edits":[]}});
    let prepared = attempt(
        &provider("claude", json!({})),
        "/v1/messages",
        payload.clone(),
    );
    assert_eq!(prepared.downstream_protocol, DownstreamProtocol::Native);
    let output = body(prepared);
    assert_eq!(output["messages"], payload["messages"]);
    assert_eq!(output["context_management"], payload["context_management"]);
    assert_eq!(output["model"], "actual");
}

#[test]
fn gateway_features_messages_convert_tools_images_and_responses_wire() {
    let payload = json!({"model":"sample","stream":true,"max_tokens":128,"system":[{"type":"text","text":"instruction"}],"messages":[{"role":"assistant","content":[{"type":"tool_use","id":"call_1","name":"lookup","input":{"city":"Shanghai"}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"call_1","content":"ok"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"AA=="}}]}],"tools":[{"name":"lookup","input_schema":{"type":"object"}}],"tool_choice":{"type":"tool","name":"lookup","disable_parallel_tool_use":true}});
    let prepared = attempt(&provider("gpt", json!({})), "/v1/messages", payload.clone());
    assert_eq!(
        prepared.downstream_protocol,
        DownstreamProtocol::MessagesCompat
    );
    assert!(prepared.url.ends_with("/v1/chat/completions"));
    let output = body(prepared);
    assert_eq!(output["messages"][0]["role"], "system");
    assert_eq!(
        output["messages"][1]["tool_calls"][0]["function"]["arguments"],
        "{\"city\":\"Shanghai\"}"
    );
    assert_eq!(output["messages"][2]["tool_call_id"], "call_1");
    assert_eq!(
        output["messages"][3]["content"][0]["image_url"]["url"],
        "data:image/png;base64,AA=="
    );
    assert_eq!(output["parallel_tool_calls"], false);
    let mut responses = (*provider("gpt", json!({}))).clone();
    responses.base_url = "https://example.test/v1/responses".into();
    let prepared = attempt(&responses, "/v1/messages", payload);
    assert_eq!(prepared.adapter, ResponseAdapter::ResponsesToChat);
    assert!(body(prepared).get("input").is_some());
}

#[test]
fn gateway_features_gemini_converts_function_turns_without_losing_ids() {
    let request = json!({"systemInstruction":{"parts":[{"text":"rules"}]},"contents":[{"role":"model","parts":[{"functionCall":{"name":"lookup","args":{"id":7}}}]},{"role":"user","parts":[{"functionResponse":{"name":"lookup","response":{"ok":true}}},{"text":"continue"}]}],"tools":[{"functionDeclarations":[{"name":"lookup","parameters":{"type":"object"}}]}],"generationConfig":{"maxOutputTokens":99}});
    let chat = gemini_to_chat_request(&request, "actual", false).unwrap();
    assert_eq!(
        chat["messages"][1]["tool_calls"][0]["id"],
        chat["messages"][2]["tool_call_id"]
    );
    assert_eq!(chat["messages"][3]["content"][0]["text"], "continue");
    assert_eq!(chat["max_tokens"], 99);
    let prepared = attempt(
        &provider("claude", json!({})),
        "/v1beta/models/sample:generateContent",
        request,
    );
    assert_eq!(prepared.adapter, ResponseAdapter::ClaudeToChat);
    assert_eq!(
        prepared.downstream_protocol,
        DownstreamProtocol::GeminiCompat
    );
    assert_eq!(
        body(prepared)["messages"][0]["content"][0]["type"],
        "tool_use"
    );
}

#[test]
fn gateway_features_unsupported_cross_protocol_content_is_not_silently_dropped() {
    assert!(messages_to_chat(&json!({"messages":[{"role":"user","content":[{"type":"document","source":{"data":"secret"}}]}]}), "sample").is_err());
    assert!(gemini_to_chat_request(
        &json!({"contents":[{"parts":[{"fileData":{"fileUri":"gs://file"}}]}]}),
        "sample",
        false
    )
    .is_err());
    let malformed = json!({"choices":[{"message":{"tool_calls":[{"id":"call","function":{"name":"lookup","arguments":"{"}}]}}]});
    assert!(chat_response(&malformed, DownstreamProtocol::MessagesCompat, "sample").is_err());
}

#[test]
fn gateway_features_messages_stream_balances_blocks_and_reports_final_usage() {
    let mut stream = DialectStream::new(DownstreamProtocol::MessagesCompat, "public".into());
    let mut events = stream.push(
        &json!({"id":"upstream","choices":[{"index":0,"delta":{"reasoning_content":"think"}}]}),
    );
    events.extend(stream.push(&json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call","function":{"name":"lookup","arguments":"{\"id\":"}}]}}]})));
    events.extend(stream.push(&json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"7}"}}]},"finish_reason":"tool_calls"}]})));
    stream.push(&json!({"choices":[],"usage":{"prompt_tokens":20,"completion_tokens":8,"prompt_tokens_details":{"cached_tokens":12}}}));
    events.extend(stream.finish());
    assert_eq!(events[0]["type"], "message_start");
    assert_eq!(
        events
            .iter()
            .filter(|event| event["type"] == "content_block_start")
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event["type"] == "content_block_stop")
            .count(),
        2
    );
    let terminal = events
        .iter()
        .find(|event| event["type"] == "message_delta")
        .unwrap();
    assert_eq!(terminal["delta"]["stop_reason"], "tool_use");
    assert_eq!(terminal["usage"]["input_tokens"], 8);
    assert_eq!(terminal["usage"]["cache_read_input_tokens"], 12);
    assert_eq!(events.last().unwrap()["type"], "message_stop");
    assert!(stream.finish().is_empty());
}

#[test]
fn gateway_features_gemini_stream_accumulates_fragmented_function_arguments() {
    let mut stream = DialectStream::new(DownstreamProtocol::GeminiCompat, "public".into());
    stream.push(&json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call","function":{"name":"lookup","arguments":"{\"id\":"}}]}}]}));
    stream.push(&json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"7}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5}}));
    let events = stream.finish();
    assert_eq!(
        events[0]["candidates"][0]["content"]["parts"][0]["functionCall"]["args"]["id"],
        7
    );
    assert_eq!(events[0]["usageMetadata"]["totalTokenCount"], 5);
    assert_eq!(events[0]["candidates"][0]["finishReason"], "STOP");
}

#[test]
fn gateway_features_messages_blocks_are_sequential_with_parallel_fragmented_tools() {
    let mut stream = DialectStream::new(DownstreamProtocol::MessagesCompat, "public".into());
    let mut events =
        stream.push(&json!({"id":"upstream","choices":[{"delta":{"reasoning_content":"think"}}]}));
    events.extend(stream.push(&json!({"choices":[{"delta":{"content":"answer"}}]})));
    events.extend(stream.push(&json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"first","function":{"name":"look","arguments":"{\"id\":"}},{"index":1,"id":"second","function":{"name":"other","arguments":"{}"}}]}}]})));
    events.extend(stream.push(&json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"up","arguments":"7}"}}]},"finish_reason":"tool_calls"}]})));
    events.extend(stream.finish());
    let mut active = None;
    let mut tools = Vec::new();
    for event in &events {
        match event["type"].as_str().unwrap() {
            "content_block_start" => {
                assert!(active.is_none());
                active = event["index"].as_u64();
                if event["content_block"]["type"] == "tool_use" {
                    tools.push(event["content_block"].clone());
                }
            }
            "content_block_delta" => assert_eq!(event["index"].as_u64(), active),
            "content_block_stop" => {
                assert_eq!(event["index"].as_u64(), active);
                active = None;
            }
            _ => {}
        }
    }
    assert!(active.is_none());
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0]["name"], "lookup");
    assert_eq!(tools[1]["id"], "second");
    assert!(events
        .iter()
        .any(|event| event.pointer("/delta/partial_json") == Some(&json!("{\"id\":7}"))));
}

#[tokio::test]
async fn gateway_features_truncated_streams_never_fabricate_completion() {
    for dialect in [
        DownstreamProtocol::MessagesCompat,
        DownstreamProtocol::GeminiCompat,
    ] {
        let source = "data: {\"id\":\"test\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"partial\"}}]}\n\n";
        let output = translate_body(Response::new(Body::from(source)), dialect, "public".into());
        let bytes = axum::body::to_bytes(output.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(text.contains("without a completion event"));
        assert!(!text.contains("message_stop"));
        assert!(!text.contains("finishReason"));
        let output = translate_body(
            Response::new(Body::from(format!("{source}data: {{"))),
            dialect,
            "public".into(),
        );
        assert!(axum::body::to_bytes(output.into_body(), usize::MAX)
            .await
            .is_err());
    }
}

#[tokio::test]
async fn gateway_features_done_ends_the_stream_without_waiting_for_transport_close() {
    let chunk = Ok::<_, io::Error>(Bytes::from_static(b"data: {\"id\":\"test\",\"choices\":[{\"delta\":{\"content\":\"done\"}}]}\n\ndata: [DONE]\n\n"));
    let source = futures_util::stream::iter([chunk]).chain(futures_util::stream::pending());
    let output = translate_body(
        Response::new(Body::from_stream(source)),
        DownstreamProtocol::MessagesCompat,
        "public".into(),
    );
    let bytes = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        axum::body::to_bytes(output.into_body(), usize::MAX),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(String::from_utf8(bytes.to_vec())
        .unwrap()
        .contains("message_stop"));
}

#[tokio::test]
async fn gateway_features_stream_wrapper_handles_fragmented_sse_and_errors() {
    let source = "data: {\"id\":\"test\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"你好\"}}]}\r\n\r\ndata: {\"error\":{\"message\":\"broken\"}}\n\ndata: [DONE]\n\n";
    let chunks = source
        .as_bytes()
        .iter()
        .map(|byte| Ok::<_, io::Error>(Bytes::from(vec![*byte])))
        .collect::<Vec<_>>();
    let output = translate_body(
        Response::new(Body::from_stream(futures_util::stream::iter(chunks))),
        DownstreamProtocol::MessagesCompat,
        "sample".into(),
    );
    let bytes = axum::body::to_bytes(output.into_body(), 65536)
        .await
        .unwrap();
    let wire = std::str::from_utf8(&bytes).unwrap();
    assert!(wire.contains("你好"));
    assert!(wire.contains("event: error"));
    assert!(!wire.contains("message_stop"));
}

#[test]
fn gateway_features_virtual_models_compile_to_ordered_group_filtered_routes() {
    let config = json!({"providers":[{"provider":"first","base_url":"https://a.test/v1","api":"one","model":[{"real-a":"a"},{"real-b":"alternate"}],"groups":["prod"],"preferences":{"usage_pricing":{"models":{"a":{"input":2}}}}},{"provider":"second","base_url":"https://b.test/v1","api":"two","model":["b"],"groups":["dev"]}],"virtual_models":{"assistant":[{"provider":"first","model":"a"},{"virtual":"fallback"}],"fallback":[{"provider":"second","model":"b"},{"provider":"first","model":"alternate","on_status":[429,502]}]}});
    let compiled: Value = serde_json::from_slice(
        &crate::config::compiler::compile_snapshot_bytes(
            &serde_json::to_vec(&config).unwrap(),
            true,
        )
        .unwrap(),
    )
    .unwrap();
    let providers = compiled["providers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| {
            runtime_provider(
                serde_json::from_value(value.clone()).unwrap(),
                Arc::new(AtomicUsize::new(0)),
            )
        })
        .collect::<Vec<_>>();
    let by_name: HashMap<_, _> = providers
        .iter()
        .map(|provider| (provider.name.to_string(), provider.clone()))
        .collect();
    let preferences = compiled["preferences"].as_object().unwrap().clone();
    let routes = crate::config::virtual_models::runtime(&preferences, &by_name);
    assert_eq!(routes["assistant"].len(), 3);
    assert_eq!(routes["assistant"][2].models["assistant"], "real-b");
    assert_eq!(
        routes["assistant"][0].preferences["usage_pricing"]["models"]["assistant"]["input"],
        2
    );
    let key = ApiKey {
        token: "caller".into(),
        model_rules: Arc::new(vec!["<assistant>".into()]),
        role: "user".into(),
        preferences: Arc::new(serde_json::from_value(json!({"groups":["prod"]})).unwrap()),
        weights: Arc::new(Default::default()),
        native_supported: true,
    };
    let snapshot = Snapshot {
        revision: "test".into(),
        preferences: Arc::new(preferences),
        api_keys: Arc::new(HashMap::new()),
        api_key_order: Arc::new(Vec::new()),
        providers: Arc::new(providers),
        providers_by_name: Arc::new(by_name),
        virtual_models: Arc::new(routes),
        api_config: Arc::new(config),
    };
    let entries = crate::routing::catalog::model_entries(&snapshot, &key, "assistant");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].0.models["assistant"], "real-a");
    assert_eq!(entries[1].0.models["assistant"], "real-b");
    assert!(!crate::routing::access::virtual_target_accepts(
        &entries[1].0,
        400
    ));
    assert!(crate::routing::access::virtual_fallback_on(
        &[entries[1].0.clone()],
        429
    ));
}

#[test]
fn gateway_features_virtual_cycles_unknown_targets_and_disabled_channels() {
    let mut config = json!({"providers":[{"provider":"first","base_url":"https://a.test/v1","api":"one","model":["a"]}],"virtual_models":{"loop":[{"virtual":"loop"}]}});
    assert!(crate::config::virtual_models::compile(&config)
        .unwrap_err()
        .contains("cycle"));
    config["virtual_models"] = json!({"bad":[{"provider":"first","model":"absent"}]});
    assert!(crate::config::virtual_models::compile(&config).is_err());
    config["virtual_models"] = json!({"alias":[{"provider":"first","model":"a"}]});
    config["providers"][0]["enabled"] = json!(false);
    let compiled: Value = serde_json::from_slice(
        &crate::config::compiler::compile_snapshot_bytes(
            &serde_json::to_vec(&config).unwrap(),
            true,
        )
        .unwrap(),
    )
    .unwrap();
    assert!(compiled["providers"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn gateway_features_byok_never_touches_pool_cooldowns_or_credentials() {
    let provider = provider(
        "gpt",
        json!({"allow_byok":true,"headers":{"Authorization":"Bearer shared-override"},"api_key_cooldown_period":60}),
    );
    let mut headers = HeaderMap::new();
    headers.insert("x-miora-byok-provider", "primary".parse().unwrap());
    headers.insert("x-miora-byok-api-key", "caller-key".parse().unwrap());
    let routes = crate::routing::access::caller_credentials(&headers, vec![provider.clone()])
        .unwrap_or_else(|_| panic!("BYOK resolution failed"));
    let selected = &routes[0];
    let runtime = crate::runtime::state::GatewayRuntime::new();
    assert!(
        matches!(runtime.select_provider_key(selected,"actual").await,crate::runtime::scheduling::ProviderKeySelection::Selected(key) if key == "caller-key")
    );
    runtime
        .cool_failed_route(crate::routing::types::FailedRoute {
            provider: selected,
            key: "caller-key",
            original_model: "actual",
            has_alternative: true,
            status: 429,
            detail: "rate limit",
            provider_model_unavailable: false,
            force_quota_cooldown: true,
        })
        .await;
    assert!(runtime.scheduling.key_cooldowns.lock().await.is_empty());
    assert!(runtime.scheduling.channel_cooldowns.lock().await.is_empty());
    assert!(runtime.scheduling.provider_windows.lock().await.is_empty());
    assert!(runtime.scheduling.key_failures.lock().await.is_empty());
    assert_eq!(provider.api_keys.as_ref(), &["shared-key"]);
    assert_eq!(selected.credential.statistics_key("caller-key"), "BYOK");
    assert_eq!(
        provider.credential.statistics_key("shared-key"),
        "shared-key"
    );
    assert_eq!(
        provider.cursor.load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    let prepared = attempt(
        selected,
        "/v1/chat/completions",
        json!({"messages":[{"role":"user","content":"hi"}]}),
    );
    assert_eq!(prepared.headers["authorization"], "Bearer caller-key");
    assert!(crate::routing::access::caller_credentials(&headers, Vec::new()).is_err());
}

#[test]
fn gateway_features_reasoning_suffixes_and_key_switches_are_compiled_once() {
    let source = json!({"providers":[{"provider":"primary","base_url":"https://a.test/v1","api":["one","two","three"],"disabled_key_positions":[2],"model":["sample"],"preferences":{"reasoning_suffixes":{"-high":{"effort":"high","budget_tokens":2048}},"reasoning_history":"strip"}}]});
    let compiled: Value = serde_json::from_slice(
        &crate::config::compiler::compile_snapshot_bytes(
            &serde_json::to_vec(&source).unwrap(),
            true,
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(compiled["providers"][0]["api"], json!(["one", "three"]));
    assert_eq!(compiled["providers"][0]["models"]["sample-high"], "sample");
    let provider = runtime_provider(
        serde_json::from_value(compiled["providers"][0].clone()).unwrap(),
        Arc::new(AtomicUsize::new(0)),
    );
    let mut body = json!({"messages":[{"role":"assistant","content":"answer","reasoning_content":"old thought"}]});
    crate::providers::reliability::apply(
        &provider,
        "sample-high",
        "/v1/chat/completions",
        &mut body,
    )
    .unwrap();
    assert_eq!(body["reasoning_effort"], "high");
    assert!(body["messages"][0].get("reasoning_content").is_none());
    body["reasoning_effort"] = json!("low");
    crate::providers::reliability::apply(
        &provider,
        "sample-high",
        "/v1/chat/completions",
        &mut body,
    )
    .unwrap();
    assert_eq!(body["reasoning_effort"], "low");
    assert!(crate::providers::reliability::blocked(
        &json!({"promptFeedback":{"blockReason":"SAFETY"}})
    ));
    assert!(!crate::providers::reliability::has_output(
        &json!({"choices":[{"message":{"content":""},"finish_reason":"stop"}]})
    ));
}
