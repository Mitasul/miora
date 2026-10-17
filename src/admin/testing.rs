use super::{bounded_body, error, management};
use crate::config::snapshot::{ApiKey, Snapshot};
use crate::runtime::context::AppState;
use axum::body::{to_bytes, Body};
use axum::extract::Request;
use axum::http::{Response, StatusCode, Uri};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};

type ApiResult = Result<Value, Response<Body>>;

fn selected_key(snapshot: &Snapshot, position: usize) -> Result<Arc<ApiKey>, Response<Body>> {
    let token = position.checked_sub(1).and_then(|index| {
        snapshot
            .api_config
            .get("api_keys")?
            .as_array()?
            .get(index)?
            .get("api")?
            .as_str()
    });
    token
        .and_then(|token| snapshot.api_keys.get(token))
        .cloned()
        .ok_or_else(|| {
            error(
                StatusCode::FORBIDDEN,
                "test_key_unavailable",
                "Select an enabled API key",
            )
        })
}

fn endpoint_allowed(endpoint: &str) -> bool {
    matches!(endpoint, "/v1/chat/completions" | "/v1/responses")
}

pub(super) async fn models(state: &AppState, uri: &Uri) -> ApiResult {
    let params: std::collections::HashMap<_, _> =
        url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes())
            .into_owned()
            .collect();
    let position: usize = params
        .get("position")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let endpoint = params
        .get("endpoint")
        .map(String::as_str)
        .unwrap_or("/v1/chat/completions");
    if !endpoint_allowed(endpoint) {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "invalid_test_endpoint",
            "Unsupported test endpoint",
        ));
    }
    let snapshot = state
        .runtime
        .snapshot()
        .await
        .ok_or_else(management::unavailable)?;
    if position == 0 {
        let mut rows = Vec::new();
        for provider in snapshot.providers.iter() {
            if !crate::routing::filters::provider_accepts_endpoint(provider, endpoint) {
                continue;
            }
            for model in provider.models.keys() {
                rows.push(json!({"provider": provider.name.as_ref(), "model": model}));
            }
        }
        return Ok(json!({"models": rows, "targeted": true}));
    }
    let key = selected_key(&snapshot, position)?;
    let rows: Vec<_> = crate::routing::catalog::ordered_entries(&snapshot, &key)
        .into_iter()
        .filter(|(provider, _)| {
            crate::routing::filters::provider_accepts_endpoint(provider, endpoint)
                && crate::control::channels::temporary_allowed(provider, &key)
                && !provider
                    .excluded_endpoints
                    .iter()
                    .any(|excluded| excluded.trim_end_matches('/') == endpoint)
        })
        .map(|(provider, model)| json!({"provider":provider.name.as_ref(),"model":model}))
        .collect();
    Ok(json!({"models":rows,"targeted":true}))
}

#[derive(Deserialize)]
struct TestRequest {
    #[serde(default)]
    position: usize,
    #[serde(default = "default_test_endpoint")]
    endpoint: String,
    #[serde(default)]
    provider: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    channel: Option<Value>,
    #[serde(default)]
    key_position: Option<usize>,
    #[serde(default)]
    direct: bool,
    #[serde(default = "empty_json_object")]
    request: Value,
}

fn default_test_endpoint() -> String {
    "/v1/chat/completions".into()
}

fn empty_json_object() -> Value {
    json!({})
}

fn validate(input: &TestRequest) -> Result<(), Response<Body>> {
    validate_mode(input, false)
}

fn validate_mode(input: &TestRequest, allow_stream: bool) -> Result<(), Response<Body>> {
    if !endpoint_allowed(&input.endpoint)
        || !input.request.is_object()
        || (!allow_stream && input.request.get("stream").and_then(Value::as_bool) == Some(true))
        || input
            .request
            .get("model")
            .and_then(Value::as_str)
            .is_none_or(|model| model.trim().is_empty())
    {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "invalid_model_test",
            "Provide a supported endpoint, model and a non-streaming JSON request",
        ));
    }
    Ok(())
}

pub(super) async fn stream(state: &AppState, request: Request) -> Response<Body> {
    match stream_inner(state, request).await {
        Ok(response) | Err(response) => response,
    }
}

async fn stream_inner(
    state: &AppState,
    request: Request,
) -> Result<Response<Body>, Response<Body>> {
    let raw = bounded_body(request.into_body(), 262_144).await?;
    let mut input: TestRequest = serde_json::from_slice(&raw).map_err(|_| {
        error(
            StatusCode::BAD_REQUEST,
            "invalid_playground_request",
            "Expected key position, endpoint and request",
        )
    })?;
    validate_mode(&input, true)?;
    let snapshot = state
        .runtime
        .snapshot()
        .await
        .ok_or_else(management::unavailable)?;
    let key = selected_key(&snapshot, input.position)?;
    if !input.provider.is_empty()
        && (input.endpoint != "/v1/responses" || !key.role.eq_ignore_ascii_case("admin"))
    {
        return Err(error(
            StatusCode::FORBIDDEN,
            "targeted_test_requires_admin_key",
            "Exact channel tests require an administrator key and Responses",
        ));
    }
    input.request["stream"] = json!(true);
    if input.endpoint == "/v1/chat/completions" {
        input.request["stream_options"] = json!({"include_usage":true});
    }
    let mut request = Request::builder()
        .method("POST")
        .uri(&input.endpoint)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", key.token));
    if !input.provider.is_empty() {
        request = request.header("x-uni-api-provider", &input.provider);
    }
    let request = request
        .body(Body::from(
            serde_json::to_vec(&input.request).map_err(|_| management::unavailable())?,
        ))
        .map_err(|_| {
            error(
                StatusCode::BAD_REQUEST,
                "invalid_playground_request",
                "Invalid request header",
            )
        })?;
    Ok(crate::api::handler::inference(state.clone(), request).await)
}

pub(super) async fn run(state: &AppState, request: Request) -> ApiResult {
    let raw = bounded_body(request.into_body(), 262_144).await?;
    let input: TestRequest = serde_json::from_slice(&raw).map_err(|_| {
        error(
            StatusCode::BAD_REQUEST,
            "invalid_model_test",
            "Expected API key position, endpoint and request",
        )
    })?;
    if input.direct
        || input.position == 0
        || input.channel.is_some()
        || input.key_position.is_some()
    {
        return run_direct_channel_test(state, input).await;
    }
    validate(&input)?;
    let snapshot = state
        .runtime
        .snapshot()
        .await
        .ok_or_else(management::unavailable)?;
    let key = selected_key(&snapshot, input.position)?;
    if !input.provider.is_empty()
        && (input.endpoint != "/v1/responses" || !key.role.eq_ignore_ascii_case("admin"))
    {
        return run_direct_channel_test(state, input).await;
    }
    let mut request = Request::builder()
        .method("POST")
        .uri(&input.endpoint)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", key.token));
    if !input.provider.is_empty() {
        request = request.header("x-uni-api-provider", &input.provider);
    }
    let request = request
        .body(Body::from(
            serde_json::to_vec(&input.request).map_err(|_| management::unavailable())?,
        ))
        .map_err(|_| {
            error(
                StatusCode::BAD_REQUEST,
                "invalid_model_test",
                "Invalid request header",
            )
        })?;
    let started = Instant::now();
    let operation = async {
        let response = crate::api::handler::inference(state.clone(), request).await;
        let status = response.status();
        let request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .map_err(|_| {
                error(
                    StatusCode::BAD_GATEWAY,
                    "test_response_too_large",
                    "Test response exceeded 4 MiB or could not be read",
                )
            })?;
        let body: Value = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({"text":String::from_utf8_lossy(&bytes)}));
        let body = if status.is_success() {
            management::sanitize_for_state(state, body)
        } else {
            let mut secrets = vec![key.token.as_ref()];
            for provider in snapshot.providers.iter() {
                secrets.extend(provider.api_keys.iter().map(String::as_str));
            }
            crate::observability::request_trace::error_fields(&bytes, &secrets)
        };
        Ok(
            json!({"ok":status.is_success(),"http_status":status.as_u16(),"elapsed_ms":started.elapsed().as_millis(),"request_id":request_id,"response":body}),
        )
    };
    tokio::time::timeout(Duration::from_secs(180), operation)
        .await
        .map_err(|_| {
            error(
                StatusCode::GATEWAY_TIMEOUT,
                "model_test_timeout",
                "Model test exceeded 180 seconds",
            )
        })?
}

async fn run_direct_channel_test(state: &AppState, input: TestRequest) -> ApiResult {
    if !endpoint_allowed(&input.endpoint) {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "invalid_model_test",
            "Unsupported test endpoint",
        ));
    }
    let requested_model = input
        .request
        .get("model")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .or_else(|| (!input.model.trim().is_empty()).then_some(input.model.as_str()))
        .unwrap_or("");
    let (provider, model_name) = super::channel_preview::compile_for_test(
        state,
        (!input.provider.trim().is_empty()).then_some(input.provider.trim()),
        input.channel.as_ref().unwrap_or(&json!({})),
        requested_model,
        input.key_position,
    )
    .await?;
    let original_model = provider
        .models
        .get(&model_name)
        .cloned()
        .unwrap_or_else(|| model_name.clone());
    let endpoint = if provider.engine.eq_ignore_ascii_case("codex")
        && provider
            .preferences
            .get("protocol_mode")
            .and_then(Value::as_str)
            == Some("native")
    {
        "/v1/responses"
    } else {
        input.endpoint.as_str()
    };
    let payload = direct_test_payload(&input, endpoint, &model_name)
        .map_err(|message| error(StatusCode::BAD_REQUEST, "attempt_build_failed", &message))?;
    let provider_key_raw = provider
        .api_keys
        .first()
        .cloned()
        .or_else(|| {
            crate::plugins::channel_plugin_id(&provider).map(|_| "__channel_plugin__".to_owned())
        })
        .ok_or_else(|| {
            error(
                StatusCode::BAD_REQUEST,
                "provider_has_no_api_keys",
                "该渠道尚未配置 API Key 或授权账号",
            )
        })?;
    let mut provider_key = provider_key_raw.clone();
    let mut codex_account_id = None;
    if (matches!(
        provider.engine.to_ascii_lowercase().as_str(),
        "codex" | "claude" | "antigravity"
    ) && provider_key_raw.contains(','))
        || provider_key_raw.trim_start().starts_with('{')
    {
        let auth = state
            .codex_oauth
            .resolve(
                &provider_key_raw,
                provider.preferences.get("proxy").and_then(Value::as_str),
            )
            .await
            .map_err(|message| {
                error(
                    StatusCode::UNAUTHORIZED,
                    "oauth_resolution_failed",
                    &message,
                )
            })?;
        provider_key = auth.bearer;
        codex_account_id = auth.account_id;
    }
    let uri: Uri = endpoint.parse().map_err(|_| {
        error(
            StatusCode::BAD_REQUEST,
            "invalid_model_test",
            "Invalid endpoint",
        )
    })?;
    let request_id = format!("test-{}", crate::runtime::clock::unix_millis());
    let prepared_input = crate::providers::types::PreparedInput {
        payload: Some(payload),
        replay: None,
        observation: crate::transport::spool::SpoolObservation::default(),
        default_model: String::new(),
        content_type: "application/json".into(),
    };
    let mut prepared = state
        .plugins
        .prepare(
            &provider,
            &provider_key,
            &model_name,
            &original_model,
            &axum::http::Method::POST,
            &uri,
            endpoint,
            &axum::http::HeaderMap::new(),
            &prepared_input,
            &request_id,
        )
        .map_err(|message| error(StatusCode::BAD_REQUEST, "attempt_build_failed", &message))?;
    if let Some(account_id) = codex_account_id {
        if let Ok(value) = axum::http::HeaderValue::from_str(&account_id) {
            prepared.headers.insert("chatgpt-account-id", value);
        }
    }
    let started = Instant::now();
    let attempt_id = format!("{request_id}-r1");
    let operation = async {
        let result = crate::upstream::generic::send_attempt(
            state,
            &provider,
            prepared,
            &axum::http::HeaderMap::new(),
            endpoint,
            &attempt_id,
            None,
            None,
        )
        .await;
        let mut secrets = vec![provider_key.as_str(), provider_key_raw.as_str()];
        for item in provider.api_keys.iter() {
            secrets.push(item.as_str());
        }
        match result {
            Ok(success) => {
                let status = success.status;
                let bytes = to_bytes(success.response.into_body(), 4 * 1024 * 1024)
                    .await
                    .unwrap_or_default();
                let body: Value = serde_json::from_slice(&bytes)
                    .unwrap_or_else(|_| json!({"text": String::from_utf8_lossy(&bytes)}));
                let body = management::sanitize_for_state(state, body);
                Ok(json!({
                    "ok": status.is_success(),
                    "http_status": status.as_u16(),
                    "elapsed_ms": started.elapsed().as_millis(),
                    "request_id": request_id,
                    "model": model_name,
                    "upstream_model": original_model,
                    "response": body,
                }))
            }
            Err(failure) => {
                let status = failure.status;
                let body = if let Some(resp) = failure.response {
                    let bytes = to_bytes(resp.into_body(), 4 * 1024 * 1024)
                        .await
                        .unwrap_or_default();
                    crate::observability::request_trace::error_fields(&bytes, &secrets)
                } else {
                    json!({"error": {"message": failure.detail, "type": failure.failure_reason}})
                };
                Ok(json!({
                    "ok": false,
                    "http_status": status.as_u16(),
                    "elapsed_ms": started.elapsed().as_millis(),
                    "request_id": request_id,
                    "model": model_name,
                    "upstream_model": original_model,
                    "response": body,
                }))
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(60), operation)
        .await
        .map_err(|_| {
            error(
                StatusCode::GATEWAY_TIMEOUT,
                "model_test_timeout",
                "Model test exceeded 60 seconds",
            )
        })?
}

fn direct_test_payload(
    input: &TestRequest,
    endpoint: &str,
    model_name: &str,
) -> Result<Value, String> {
    let mut payload = if input.request.as_object().is_some_and(|obj| !obj.is_empty()) {
        if endpoint == "/v1/responses" && input.endpoint == "/v1/chat/completions" {
            crate::protocols::conversion::chat_to_responses(&input.request, model_name)?
        } else {
            input.request.clone()
        }
    } else if endpoint == "/v1/responses" {
        json!({"model": model_name, "input": "hi", "max_output_tokens": 16, "stream": false})
    } else {
        json!({"model": model_name, "messages": [{"role": "user", "content": "hi"}], "max_tokens": 16, "stream": false})
    };
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("model".into(), json!(model_name));
        obj.insert("stream".into(), json!(false));
    }
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_codex_tests_convert_chat_payloads_to_responses() {
        let input: TestRequest = serde_json::from_value(json!({
            "request": {
                "model": "sample",
                "messages": [{"role": "user", "content": "test prompt"}],
                "max_tokens": 24,
                "temperature": 0.4,
                "top_p": 0.8
            }
        }))
        .unwrap();
        let payload = direct_test_payload(&input, "/v1/responses", "sample").unwrap();
        assert_eq!(payload["input"][0]["content"][0]["text"], "test prompt");
        assert_eq!(payload["max_output_tokens"], 24);
        assert_eq!(payload["temperature"], 0.4);
        assert_eq!(payload["top_p"], 0.8);
        assert_eq!(payload["stream"], false);
        assert!(payload.get("messages").is_none());
        assert!(payload.get("max_tokens").is_none());
        let chat_payload = direct_test_payload(&input, "/v1/chat/completions", "sample").unwrap();
        assert_eq!(chat_payload["messages"], input.request["messages"]);
        assert_eq!(chat_payload["max_tokens"], 24);
    }

    #[test]
    fn native_codex_tests_preserve_responses_fields_and_generate_native_defaults() {
        let input: TestRequest = serde_json::from_value(json!({
            "endpoint": "/v1/responses",
            "request": {"input": "native prompt", "max_output_tokens": 32, "vendor_field": true}
        }))
        .unwrap();
        let payload = direct_test_payload(&input, "/v1/responses", "sample").unwrap();
        assert_eq!(payload["input"], "native prompt");
        assert_eq!(payload["max_output_tokens"], 32);
        assert_eq!(payload["vendor_field"], true);
        let empty: TestRequest = serde_json::from_value(json!({})).unwrap();
        let payload = direct_test_payload(&empty, "/v1/responses", "sample").unwrap();
        assert_eq!(payload["input"], "hi");
        assert_eq!(payload["max_output_tokens"], 16);
    }

    #[test]
    fn workspace_features_only_nonstream_inference_endpoints_can_be_tested() {
        let mut input = TestRequest {
            position: 1,
            endpoint: "/v1/chat/completions".into(),
            provider: String::new(),
            model: String::new(),
            channel: None,
            key_position: None,
            direct: false,
            request: json!({"model":"sample","stream":false}),
        };
        assert!(validate(&input).is_ok());
        input.endpoint = "/admin/api/settings".into();
        assert!(validate(&input).is_err());
        input.endpoint = "/v1/responses".into();
        input.request["stream"] = json!(true);
        assert!(validate(&input).is_err());
    }
}
