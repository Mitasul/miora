//! Pure channel editor preview and source-preserving draft semantics.
use super::{bounded_body, error, management};
use crate::config::snapshot::{runtime_provider, Provider, RawProvider};
use crate::config::source::RuntimeConfigSource;
use crate::providers::parameters;
use crate::providers::types::AttemptBody;
use crate::runtime::context::AppState;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{Response, StatusCode};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

type ResultValue = Result<Value, Response<Body>>;
const MASK: &str = "********";

#[derive(Deserialize)]
struct EditorInput {
    provider: Option<String>,
    #[serde(default = "empty_object")]
    channel: Value,
    #[serde(default)]
    request: PreviewRequest,
}

#[derive(Default, Deserialize)]
struct PreviewRequest {
    endpoint: Option<String>,
    model: Option<String>,
    stream: Option<bool>,
    body: Option<Value>,
}

fn empty_object() -> Value {
    json!({})
}

pub(super) async fn handle(state: &AppState, request: Request, schema_only: bool) -> ResultValue {
    let raw = bounded_body(request.into_body(), 262_144).await?;
    let input: EditorInput = serde_json::from_slice(&raw).map_err(|_| {
        error(
            StatusCode::BAD_REQUEST,
            "invalid_preview",
            "Expected provider, a channel draft object and optional request fields",
        )
    })?;
    let (revision, config) = match &state.config_publisher.source {
        RuntimeConfigSource::File(_) => management::source_config(state).await?,
        RuntimeConfigSource::Url(_) => {
            let snapshot = state
                .runtime
                .snapshot()
                .await
                .ok_or_else(management::unavailable)?;
            (
                snapshot.revision.to_string(),
                snapshot.api_config.as_ref().clone(),
            )
        }
    };
    let original = if let Some(name) = input.provider.as_deref() {
        config
            .get("providers")
            .and_then(Value::as_array)
            .and_then(|rows| {
                rows.iter()
                    .find(|row| row.get("provider").and_then(Value::as_str) == Some(name))
            })
            .cloned()
            .ok_or_else(|| {
                error(
                    StatusCode::NOT_FOUND,
                    "channel_not_found",
                    "Saved channel not found",
                )
            })?
    } else {
        json!({})
    };
    let mut document = merge_channel(&original, &input.channel).map_err(policy_error)?;
    validate_document_policy(&document).map_err(policy_error)?;
    let saved = input.provider.is_some() && document == original;
    let request = input.request;
    let requested_model = request
        .model
        .as_deref()
        .or_else(|| {
            request
                .body
                .as_ref()
                .and_then(|body| body.get("model"))
                .and_then(Value::as_str)
        })
        .map(str::to_owned)
        .or_else(|| first_model(&document))
        .unwrap_or_else(|| "preview-model".into());
    if document.get("provider").and_then(Value::as_str).is_none() {
        document["provider"] = json!("admin-preview");
    }
    if !document
        .get("model")
        .and_then(Value::as_array)
        .is_some_and(|models| !models.is_empty())
    {
        document["model"] = json!([requested_model]);
    }
    crate::config::compiler::expand_environment(&mut document);
    let provider = compile_provider(&document)?;
    let endpoint = request
        .endpoint
        .as_deref()
        .unwrap_or("/v1/chat/completions");
    let registry = state.plugins.pin();
    let capabilities = editor_capabilities(&registry, &provider, endpoint);
    if schema_only {
        return Ok(json!({"capabilities":capabilities,"revision":revision,"saved":saved}));
    }
    if !capabilities
        .get("endpoints")
        .and_then(Value::as_array)
        .is_some_and(|paths| paths.iter().any(|path| path == endpoint))
    {
        return Err(unsupported(
            "This channel does not support preview for the selected request endpoint",
        ));
    }
    let upstream_model = provider
        .models
        .get(&requested_model)
        .cloned()
        .unwrap_or_else(|| requested_model.clone());
    let mut payload = request.body.unwrap_or_else(|| {
        if endpoint == "/v1/responses" || endpoint == "/v1/responses/compact" {
            json!({"input":"Preview input"})
        } else {
            json!({"messages":[{"role":"user","content":"Preview input"}]})
        }
    });
    let object = payload.as_object_mut().ok_or_else(|| {
        error(
            StatusCode::BAD_REQUEST,
            "invalid_preview",
            "Preview request body must be an object",
        )
    })?;
    object.insert("model".into(), json!(requested_model));
    if let Some(stream) = request.stream {
        object.insert("stream".into(), json!(stream));
    }
    let attempt = parameters::prepare_preview(&registry, &provider, &requested_model, &upstream_model, endpoint, payload)
        .map_err(|_| unsupported("An exact offline preview is unavailable for this channel, endpoint or parameter policy; no request was sent"))?;
    let report = serde_json::to_value(&attempt.policy_report)
        .map_err(|_| unsupported("Could not describe the prepared request policy"))?;
    let mut body = match &attempt.body {
        AttemptBody::Json(bytes) => serde_json::from_slice::<Value>(bytes)
            .map_err(|_| unsupported("Prepared request is not JSON"))?,
        AttemptBody::Empty => Value::Null,
        _ => {
            return Err(unsupported(
                "File and streaming upload bodies do not have an offline JSON preview",
            ))
        }
    };
    let mut credentials = Vec::new();
    collect_masked_strings(
        &document,
        &management::sanitize_for_state(state, document.clone()),
        &mut credentials,
    );
    collect_url_secrets(&document, &mut credentials);
    if let Some(id) = plugin_id(&provider) {
        if let Some(version) = registry.snapshot().versions.get(id) {
            collect_masked_strings(
                &Value::Object(version.config.clone()),
                &Value::Object(registry.redact_channel_config(id, &version.config)),
                &mut credentials,
            );
            if !version.service_auth.token.is_empty() {
                credentials.push(version.service_auth.token.clone());
            }
            redact_plugin_envelope(&registry, id, &mut body);
        }
    }
    let headers = attempt
        .headers
        .iter()
        .map(|(name, value)| {
            let secret = declared_header_secret(&document, name.as_str())
                || attempt
                    .policy_report
                    .sensitive_headers
                    .iter()
                    .any(|header| header.eq_ignore_ascii_case(name.as_str()));
            let value = if parameters::header_sensitive(name.as_str(), secret) {
                MASK.to_owned()
            } else {
                value.to_str().unwrap_or("[non-text header]").to_owned()
            };
            (name.to_string(), json!(value))
        })
        .collect::<Map<_, _>>();
    let scope = if attempt
        .plugin_lease
        .as_ref()
        .is_some_and(|lease| lease.service)
    {
        "plugin_envelope"
    } else {
        "upstream_request"
    };
    let upstream_model = attempt.wire_model.clone().unwrap_or(upstream_model);
    let mut result = json!({
        "scope":scope,"method":attempt.method.as_str(),"url":management::safe_url(&attempt.url),
        "headers":headers,"body":management::sanitize_for_state(state, body),
        "requested_model":requested_model,"upstream_model":upstream_model,
        "policy_trace":report.get("trace").cloned().unwrap_or_else(|| json!([])),
        "policy_report":report,"capabilities":capabilities,"revision":revision,"saved":saved
    });
    redact_known_strings(&mut result, &credentials);
    Ok(result)
}

fn unsupported(message: &str) -> Response<Body> {
    error(
        StatusCode::UNPROCESSABLE_ENTITY,
        "preview_unsupported",
        message,
    )
}

pub(super) fn policy_error(message: String) -> Response<Body> {
    error(
        StatusCode::BAD_REQUEST,
        "invalid_parameter_policy",
        &message,
    )
}

fn compile_provider(document: &Value) -> Result<Arc<Provider>, Response<Body>> {
    let compiled = crate::config::compiler::compile_provider(document).ok_or_else(|| {
        error(
            StatusCode::BAD_REQUEST,
            "invalid_preview",
            "Channel type and model configuration could not be prepared",
        )
    })?;
    let raw: RawProvider = serde_json::from_value(compiled).map_err(|_| {
        error(
            StatusCode::BAD_REQUEST,
            "invalid_preview",
            "Invalid channel configuration",
        )
    })?;
    Ok(runtime_provider(raw, Arc::new(AtomicUsize::new(0))))
}

pub(super) async fn compile_for_test(
    state: &AppState,
    provider_name: Option<&str>,
    draft: &Value,
    requested_model: &str,
    key_position: Option<usize>,
) -> Result<(Arc<Provider>, String), Response<Body>> {
    let (_, config) = match &state.config_publisher.source {
        RuntimeConfigSource::File(_) => management::source_config(state).await?,
        RuntimeConfigSource::Url(_) => {
            let snapshot = state
                .runtime
                .snapshot()
                .await
                .ok_or_else(management::unavailable)?;
            (
                snapshot.revision.to_string(),
                snapshot.api_config.as_ref().clone(),
            )
        }
    };
    compile_test_provider(&config, provider_name, draft, requested_model, key_position)
}

fn compile_test_provider(
    config: &Value,
    provider_name: Option<&str>,
    draft: &Value,
    requested_model: &str,
    key_position: Option<usize>,
) -> Result<(Arc<Provider>, String), Response<Body>> {
    let has_draft = draft.as_object().is_some_and(|obj| !obj.is_empty());
    let original = if let Some(name) = provider_name.filter(|s| !s.trim().is_empty()) {
        let found = config
            .get("providers")
            .and_then(Value::as_array)
            .and_then(|rows| {
                rows.iter()
                    .find(|row| row.get("provider").and_then(Value::as_str) == Some(name))
            })
            .cloned();
        match found {
            Some(row) => row,
            None if has_draft => json!({}),
            None => {
                return Err(error(
                    StatusCode::NOT_FOUND,
                    "channel_not_found",
                    "Saved channel not found",
                ))
            }
        }
    } else {
        json!({})
    };
    let mut document = if has_draft {
        merge_channel(&original, draft).map_err(policy_error)?
    } else {
        original
    };
    let model_name = if !requested_model.trim().is_empty() {
        requested_model.trim().to_owned()
    } else {
        first_model(&document).ok_or_else(|| {
            error(
                StatusCode::BAD_REQUEST,
                "invalid_model_test",
                "请先为渠道添加或选择一个模型后再测试",
            )
        })?
    };
    if document
        .get("provider")
        .and_then(Value::as_str)
        .is_none_or(|s| s.trim().is_empty())
    {
        document["provider"] = json!(provider_name.unwrap_or("admin-test"));
    }
    let has_model = document
        .get("model")
        .and_then(Value::as_array)
        .is_some_and(|models| {
            models.iter().any(|item| {
                item.as_str() == Some(model_name.as_str())
                    || item.as_object().is_some_and(|map| {
                        map.contains_key(&model_name)
                            || map.values().any(|v| v.as_str() == Some(model_name.as_str()))
                    })
            })
        });
    if !has_model {
        match document.get_mut("model").and_then(Value::as_array_mut) {
            Some(models) => models.push(json!(model_name)),
            None => document["model"] = json!([model_name]),
        }
    }
    crate::config::compiler::expand_environment(&mut document);
    if let Some(pos) = key_position.filter(|&p| p >= 1) {
        let keys = crate::config::snapshot::provider_api_keys(&document["api"]);
        let selected = keys
            .get(pos - 1)
            .map(|k| k.trim_start_matches('!').trim().to_owned())
            .filter(|k| !k.is_empty())
            .ok_or_else(|| {
                error(
                    StatusCode::BAD_REQUEST,
                    "invalid_test_key",
                    "未找到指定位置的渠道账号或凭据",
                )
            })?;
        document["api"] = json!(selected);
        document["disabled_key_positions"] = json!([]);
    } else {
        // If all keys in the channel are disabled or the channel itself is disabled,
        // still allow explicit admin testing using the first configured credential.
        let keys = crate::config::snapshot::provider_api_keys(&document["api"]);
        if !keys.is_empty() && keys.iter().all(|k| k.trim_start().starts_with('!')) {
            if let Some(first) = keys
                .first()
                .map(|k| k.trim_start_matches('!').trim().to_owned())
                .filter(|k| !k.is_empty())
            {
                document["api"] = json!(first);
            }
        }
    }
    document["enabled"] = json!(true);
    let provider = compile_provider(&document)?;
    Ok((provider, model_name))
}

fn first_model(document: &Value) -> Option<String> {
    let model = document.get("model")?.as_array()?.first()?;
    model.as_str().map(str::to_owned).or_else(|| {
        model
            .as_object()?
            .values()
            .find_map(Value::as_str)
            .map(str::to_owned)
    })
}

fn plugin_id(provider: &Provider) -> Option<&str> {
    provider
        .preferences
        .get("channel_plugin")
        .and_then(Value::as_str)
        .or_else(|| provider.engine.strip_prefix("plugin:"))
}

fn editor_capabilities(
    registry: &crate::plugins::PluginRegistry,
    provider: &Provider,
    endpoint: &str,
) -> Value {
    let mut capabilities = parameters::capabilities(provider, endpoint);
    if let Some(id) = plugin_id(provider) {
        let paths = registry
            .snapshot()
            .versions
            .get(id)
            .map(|version| {
                version
                    .manifest
                    .capabilities
                    .endpoints
                    .iter()
                    .filter(|path| {
                        parameters::capabilities(provider, path).get("body_supported")
                            == Some(&Value::Bool(true))
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        capabilities["endpoints"] = json!(paths);
        if !paths.iter().any(|path| path == endpoint) {
            capabilities["body_supported"] = json!(false);
            capabilities["preview_supported"] = json!(false);
        }
    }
    capabilities
}

fn redact_plugin_envelope(registry: &crate::plugins::PluginRegistry, id: &str, body: &mut Value) {
    if let Some(config) = body.pointer_mut("/channel/config") {
        if let Some(raw) = config.as_object() {
            let mut clean = registry.redact_channel_config(id, raw);
            // Pinned execution can outlive a later registry classification change.
            if let Some(version) = registry.snapshot().versions.get(id) {
                for field in &version.manifest.config_schema {
                    if field.secret && raw.contains_key(&field.key) {
                        clean.insert(field.key.clone(), json!(MASK));
                    }
                }
            }
            *config = Value::Object(clean);
        }
    }
}

pub(super) fn validate_document_policy(document: &Value) -> Result<(), String> {
    if let Some(policy) = document.pointer("/preferences/parameter_policy") {
        parameters::validate_policy(policy)?;
    }
    Ok(())
}

/// Omission keeps source fields. A supplied policy replaces known semantics,
/// while unknown extension metadata remains round-trippable.
pub(super) fn merge_channel(original: &Value, patch: &Value) -> Result<Value, String> {
    if !patch.is_object() {
        return Err("Channel draft must be an object".into());
    }
    let mut base = original.clone();
    if patch.get("plugin_config").is_some() && base.get("plugin_config").is_none() {
        if let Some(config) = original.pointer("/preferences/plugin_config") {
            base["plugin_config"] = config.clone();
        }
    }
    merge_value(&base, patch, "")
}

fn merge_value(old: &Value, new: &Value, path: &str) -> Result<Value, String> {
    if path == "/preferences/usage_pricing/models" {
        return Ok(new.clone());
    }
    if path == "/preferences/parameter_policy" {
        return merge_policy(old, new);
    }
    if is_mask(new) {
        return if old.is_null() {
            Err("A masked credential has no saved value; supply a value or omit the field".into())
        } else {
            Ok(old.clone())
        };
    }
    if path == "/api" && new.as_str().is_some_and(|value| value.is_empty()) && !old.is_null() {
        return Ok(old.clone());
    }
    if let Value::Object(incoming) = new {
        let mut output = old.as_object().cloned().unwrap_or_default();
        for (key, value) in incoming {
            let value = merge_value(
                output.get(key).unwrap_or(&Value::Null),
                value,
                &format!("{path}/{key}"),
            )?;
            output.insert(key.clone(), value);
        }
        Ok(Value::Object(output))
    } else {
        Ok(new.clone())
    }
}

fn merge_policy(old: &Value, new: &Value) -> Result<Value, String> {
    let Some(incoming) = new.as_object() else {
        return Ok(new.clone());
    };
    let known = ["defaults", "channel", "models", "headers", "system_prompt"];
    let mut output = old.as_object().cloned().unwrap_or_default();
    for key in known {
        output.remove(key);
    }
    // Removing a known semantic section must not erase its extension metadata.
    for (section, keys) in [
        ("channel", &["set", "remove"][..]),
        ("system_prompt", &["mode", "value"][..]),
    ] {
        if !incoming.contains_key(section) {
            if let Some(previous) = old.get(section).and_then(Value::as_object) {
                let mut metadata = previous.clone();
                for key in keys {
                    metadata.remove(*key);
                }
                if !metadata.is_empty() {
                    output.insert(section.into(), Value::Object(metadata));
                }
            }
        }
    }
    for (key, value) in incoming {
        let previous = old.get(key).unwrap_or(&Value::Null);
        let merged = match key.as_str() {
            "defaults" => replace_value(previous, value)?,
            "channel" => merge_section(previous, value, &["set", "remove"], &["set", "remove"])?,
            "system_prompt" => {
                merge_section(previous, value, &["mode", "value"], &["mode", "value"])?
            }
            "headers" if value.is_array() => {
                let mut names = std::collections::BTreeSet::new();
                let rows = value.as_array().unwrap().iter().map(|row| {
                    let name = row.get("name").and_then(Value::as_str).unwrap_or("");
                    if !names.insert(name.to_ascii_lowercase()) {
                        return Err("Header names must be unique, including disabled rows (case-insensitive)".into());
                    }
                    let matches = previous.as_array().into_iter().flatten().filter(|old| {
                        old.get("name").and_then(Value::as_str).is_some_and(|old| old.eq_ignore_ascii_case(name))
                    }).collect::<Vec<_>>();
                    if matches.len() > 1 && row.get("mode").and_then(Value::as_str) == Some("set")
                        && row.get("value").map_or(true, is_mask)
                    {
                        return Err("Saved header name is ambiguous; provide an explicit value or remove duplicate rows before reusing its secret".into());
                    }
                    let old_row = if matches.len() == 1 { matches[0] } else { &Value::Null };
                    merge_header(old_row, row)
                }).collect::<Result<Vec<_>, String>>()?;
                Value::Array(rows)
            }
            "models" if value.is_array() => {
                let rows = value
                    .as_array()
                    .unwrap()
                    .iter()
                    .enumerate()
                    .map(|(index, row)| {
                        let matches = previous.as_array().into_iter().flatten().filter(|old| old.pointer("/match/requested_model") == row.pointer("/match/requested_model")).collect::<Vec<_>>();
                        if matches.len() > 1 && has_mask(row) {
                            return Err("Saved model rules are ambiguous for masked values; provide explicit values or consolidate rules for that model".to_owned());
                        }
                        let positional = previous.get(index).filter(|old| old.pointer("/match/requested_model") == row.pointer("/match/requested_model"));
                        let old_row = positional.or_else(|| matches.first().copied()).unwrap_or(&Value::Null);
                        merge_section(
                            old_row,
                            row,
                            &["match", "set", "remove"],
                            &["set", "remove"],
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Value::Array(rows)
            }
            _ => merge_value(previous, value, "/policy_extension")?,
        };
        output.insert(key.clone(), merged);
    }
    Ok(Value::Object(output))
}

fn merge_section(
    old: &Value,
    new: &Value,
    known: &[&str],
    replace: &[&str],
) -> Result<Value, String> {
    let Some(incoming) = new.as_object() else {
        return Ok(new.clone());
    };
    let mut output = old.as_object().cloned().unwrap_or_default();
    for key in known {
        output.remove(*key);
    }
    for (key, value) in incoming {
        output.insert(
            key.clone(),
            if replace.contains(&key.as_str()) {
                replace_value(old.get(key).unwrap_or(&Value::Null), value)?
            } else {
                merge_value(old.get(key).unwrap_or(&Value::Null), value, "")?
            },
        );
    }
    Ok(Value::Object(output))
}

fn replace_value(old: &Value, new: &Value) -> Result<Value, String> {
    if is_mask(new) {
        return if old.is_null() {
            Err("A masked credential has no saved value; supply a value or omit the field".into())
        } else {
            Ok(old.clone())
        };
    }
    match new {
        Value::Object(values) => Ok(Value::Object(
            values
                .iter()
                .map(|(key, value)| {
                    Ok((
                        key.clone(),
                        replace_value(old.get(key).unwrap_or(&Value::Null), value)?,
                    ))
                })
                .collect::<Result<Map<_, _>, String>>()?,
        )),
        Value::Array(values) => Ok(Value::Array(
            values
                .iter()
                .enumerate()
                .map(|(index, value)| replace_value(old.get(index).unwrap_or(&Value::Null), value))
                .collect::<Result<Vec<_>, _>>()?,
        )),
        _ => Ok(new.clone()),
    }
}

fn merge_header(old: &Value, new: &Value) -> Result<Value, String> {
    let mut row = merge_section(
        old,
        new,
        &["name", "mode", "enabled", "secret", "value"],
        &["name", "mode", "enabled", "secret", "value"],
    )?;
    let Some(object) = row.as_object_mut() else {
        return Ok(row);
    };
    for field in ["secret", "enabled"] {
        if !object.contains_key(field) {
            if let Some(value) = old.get(field) {
                object.insert(field.into(), value.clone());
            }
        }
    }
    let mode = object
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("inherit");
    if mode != "set" {
        object.remove("value");
        return Ok(row);
    }
    if !object.contains_key("value") || object.get("value").is_some_and(is_mask) {
        let value = old.get("value").filter(|value| !is_mask(value)).ok_or(
            "A new set-mode header requires a value; omit the row to inherit or use remove mode",
        )?;
        object.insert("value".into(), value.clone());
    }
    Ok(row)
}

fn has_mask(value: &Value) -> bool {
    is_mask(value)
        || match value {
            Value::Object(values) => values.values().any(has_mask),
            Value::Array(values) => values.iter().any(has_mask),
            _ => false,
        }
}

fn is_mask(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|value| matches!(value, "***" | "********" | "[REDACTED]"))
}

fn declared_header_secret(document: &Value, name: &str) -> bool {
    document
        .pointer("/preferences/parameter_policy/headers")
        .and_then(Value::as_array)
        .is_some_and(|headers| {
            headers.iter().any(|header| {
                header
                    .get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|value| value.eq_ignore_ascii_case(name))
                    && header.get("secret") == Some(&Value::Bool(true))
            })
        })
}

fn collect_masked_strings(raw: &Value, redacted: &Value, out: &mut Vec<String>) {
    if is_mask(redacted) && raw != redacted {
        collect_strings(raw, out);
    } else {
        match raw {
            Value::Object(values) => {
                for (key, value) in values {
                    if let Some(clean) = redacted.get(key) {
                        collect_masked_strings(value, clean, out);
                    }
                }
            }
            Value::Array(values) => {
                for (index, value) in values.iter().enumerate() {
                    if let Some(clean) = redacted.get(index) {
                        collect_masked_strings(value, clean, out);
                    }
                }
            }
            _ => {}
        }
    }
}

fn collect_strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(value) if !value.is_empty() && !out.contains(value) => {
            out.push(value.clone())
        }
        Value::Object(values) => {
            for value in values.values() {
                collect_strings(value, out);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_strings(value, out);
            }
        }
        _ => {}
    }
}

fn collect_url_secrets(document: &Value, out: &mut Vec<String>) {
    for value in [
        document.get("base_url"),
        document.pointer("/preferences/proxy"),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(url) = value.as_str().and_then(|value| url::Url::parse(value).ok()) {
            if let Some(password) = url.password() {
                if !password.is_empty() {
                    out.push(password.to_owned());
                }
            }
            for (name, value) in url.query_pairs() {
                if (parameters::header_sensitive(&name, false) || name.eq_ignore_ascii_case("key"))
                    && !value.is_empty()
                {
                    out.push(value.into_owned());
                }
            }
        }
    }
}

fn redact_known_strings(value: &mut Value, credentials: &[String]) {
    match value {
        Value::String(text) => {
            for credential in credentials {
                if !credential.is_empty() {
                    *text = text.replace(credential, MASK);
                    let encoded: String =
                        url::form_urlencoded::byte_serialize(credential.as_bytes()).collect();
                    if encoded != *credential {
                        *text = text.replace(&encoded, MASK);
                    }
                }
            }
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                redact_known_strings(value, credentials);
            }
        }
        Value::Array(values) => {
            for value in values {
                redact_known_strings(value, credentials);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_account_tests_use_source_positions_without_disabling_the_selected_key() {
        let config = json!({"providers": [{
            "provider": "channel",
            "engine": "gpt",
            "base_url": "https://example.test/v1",
            "api": ["first-key", "second-key"],
            "disabled_key_positions": [1],
            "model": ["sample"]
        }]});
        for (position, expected) in [(1, "first-key"), (2, "second-key")] {
            let (provider, model) = compile_test_provider(
                &config,
                Some("channel"),
                &json!({}),
                "sample",
                Some(position),
            )
            .unwrap();
            assert_eq!(provider.api_keys.as_ref(), &[expected.to_owned()]);
            assert_eq!(model, "sample");
        }
        assert_eq!(config["providers"][0]["disabled_key_positions"], json!([1]));
    }

    #[test]
    fn model_price_catalog_replacement_removes_old_aliases_without_changing_defaults() {
        let original = json!({"api":"private-key","preferences":{"usage_pricing":{"input":2,"models":{"before":{"input":3},"other":{"output":5}}},"unrelated":true}});
        let patch = json!({"preferences":{"usage_pricing":{"models":{"after":{"input":3},"other":{"output":5}}}}});
        let merged = merge_channel(&original, &patch).unwrap();
        assert_eq!(merged["preferences"]["usage_pricing"]["input"], 2);
        assert!(merged["preferences"]["usage_pricing"]["models"].get("before").is_none());
        assert_eq!(merged["preferences"]["usage_pricing"]["models"]["after"]["input"], 3);
        assert_eq!(merged["preferences"]["unrelated"], true);
        assert_eq!(merged["api"], "private-key");
        let cleared = merge_channel(&merged, &json!({"preferences":{"usage_pricing":{"models":{}}}})).unwrap();
        assert_eq!(cleared["preferences"]["usage_pricing"]["models"], json!({}));
    }

    #[test]
    fn draft_policy_keeps_falsy_values_unknown_metadata_and_explicit_inheritance() {
        let original = json!({"vendor_extra":{"keep":true},"preferences":{"other":7,"parameter_policy":{"vendor_meta":{"keep":1},"defaults":{"old":9},"channel":{"vendor_note":"keep","set":{"temperature":1,"obsolete":3}}}}});
        let patch = json!({"preferences":{"parameter_policy":{"channel":{"set":{"temperature":0,"enabled":false,"text":"","nullable":null},"remove":["/old"]}}}});
        let merged = merge_channel(&original, &patch).unwrap();
        assert_eq!(merged["vendor_extra"]["keep"], true);
        assert_eq!(merged["preferences"]["other"], 7);
        let policy = &merged["preferences"]["parameter_policy"];
        assert_eq!(policy["vendor_meta"]["keep"], 1);
        assert!(policy.get("defaults").is_none());
        assert_eq!(policy["channel"]["vendor_note"], "keep");
        assert!(policy["channel"]["set"].get("obsolete").is_none());
        assert_eq!(
            policy["channel"]["set"],
            patch["preferences"]["parameter_policy"]["channel"]["set"]
        );
        assert_eq!(
            original["preferences"]["parameter_policy"]["channel"]["set"]["temperature"],
            1
        );
    }

    #[test]
    fn plugin_envelope_redaction_covers_history_and_non_string_secrets() {
        use crate::plugins::registry::{PluginRegistry, Registration, TrustPolicy};
        let registry = PluginRegistry::open(
            None,
            TrustPolicy {
                allowed_origins: Vec::new(),
                allow_loopback_http: true,
            },
        )
        .unwrap();
        let registration = |revision, secret| -> Registration {
            serde_json::from_value(json!({"expected_revision":revision,"endpoint":"http://127.0.0.1:1/plugin","manifest":{"id":"preview-fixture","name":"Preview fixture","version":format!("{revision}"),"api_version":1,"capabilities":{"endpoints":["/v1/chat/completions"]},"config_schema":[{"key":"pin","type":"integer","secret":secret},{"key":"flag","type":"boolean","secret":secret},{"key":"bag","type":"object","secret":secret},{"key":"label","type":"string"}]},"config":{"pin":1234,"flag":false,"bag":{"value":"private"},"label":"public"}})).unwrap()
        };
        registry.register(registration(0, false)).unwrap();
        let pinned = registry.pin();
        registry.register(registration(1, true)).unwrap();
        registry
            .change("preview-fixture", 2, None, Some(1))
            .unwrap();
        let original = json!({"channel":{"config":{"pin":1234,"flag":false,"bag":{"value":"private"},"label":"public"}},"request":{"body":{"temperature":0}}});
        let mut body = original.clone();
        redact_plugin_envelope(&pinned, "preview-fixture", &mut body);
        assert!(body["channel"]["config"]["pin"]
            .as_str()
            .is_some_and(|v| v == "***" || v == MASK));
        assert!(body["channel"]["config"]["flag"].is_string());
        assert!(body["channel"]["config"]["bag"].is_string());
        assert_eq!(body["channel"]["config"]["label"], "public");
        assert_eq!(body["request"]["body"]["temperature"], 0);
        assert_eq!(original["channel"]["config"]["pin"], 1234);
    }

    #[test]
    fn duplicate_header_names_never_select_the_wrong_saved_secret() {
        let original = json!({"preferences":{"parameter_policy":{"headers":[
            {"name":"Authorization","mode":"set","value":"first","enabled":false},
            {"name":"authorization","mode":"set","value":"second","enabled":true}
        ]}}});
        let masked = json!({"preferences":{"parameter_policy":{"headers":[{"name":"AUTHORIZATION","mode":"set"}]}}});
        assert!(merge_channel(&original, &masked)
            .unwrap_err()
            .contains("ambiguous"));
        let explicit = json!({"preferences":{"parameter_policy":{"headers":[{"name":"Authorization","mode":"set","value":"replacement"}]}}});
        assert_eq!(
            merge_channel(&original, &explicit)
                .unwrap()
                .pointer("/preferences/parameter_policy/headers/0/value"),
            Some(&json!("replacement"))
        );
        assert!(merge_channel(&json!({}), &original)
            .unwrap_err()
            .contains("unique"));
    }

    #[test]
    fn header_omission_masks_empty_and_removal_are_distinct() {
        let original = json!({"preferences":{"parameter_policy":{"headers":[{"name":"Authorization","mode":"set","value":"private-value","secret":false,"extension":1}]}}});
        for value in [None, Some(json!("***")), Some(json!("********"))] {
            let mut row = json!({"name":"authorization","mode":"set","secret":false});
            if let Some(value) = value {
                row["value"] = value;
            }
            let merged = merge_channel(
                &original,
                &json!({"preferences":{"parameter_policy":{"headers":[row]}}}),
            )
            .unwrap();
            assert_eq!(
                merged.pointer("/preferences/parameter_policy/headers/0/value"),
                Some(&json!("private-value"))
            );
            assert_eq!(
                merged.pointer("/preferences/parameter_policy/headers/0/extension"),
                Some(&json!(1))
            );
        }
        let cleared = merge_channel(&original, &json!({"preferences":{"parameter_policy":{"headers":[{"name":"Authorization","mode":"set","value":""}]}}})).unwrap();
        assert_eq!(
            cleared.pointer("/preferences/parameter_policy/headers/0/value"),
            Some(&json!(""))
        );
        let removed = merge_channel(&original, &json!({"preferences":{"parameter_policy":{"headers":[{"name":"Authorization","mode":"remove"}]}}})).unwrap();
        assert!(removed
            .pointer("/preferences/parameter_policy/headers/0/value")
            .is_none());
        assert!(merge_channel(
            &json!({}),
            &json!({"preferences":{"parameter_policy":{"headers":[{"name":"X-New","mode":"set"}]}}})
        )
        .is_err());
    }
}
