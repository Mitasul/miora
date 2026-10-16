//! Shared production parameter policy. The admin preview calls the same builders.
//! Unknown policy extension fields remain stored JSON; only documented sections
//! are interpreted. Reports contain paths/sources/actions, never field values.
use crate::config::snapshot::Provider;
use crate::providers::types::{
    AttemptBody, DownstreamProtocol, PreparedAttempt, PreparedInput, ResponseAdapter,
};
use axum::http::{HeaderMap, Method};
use serde::Serialize;
use serde_json::{json, Value};

mod policy;
pub(crate) use policy::{
    apply_body, apply_headers, apply_native_headers, capture_origin, validate_policy,
};
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Profile {
    Chat,
    Responses,
    Codex,
    Claude,
    Gemini,
    Unsupported,
}
impl Profile {
    fn name(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Responses => "responses",
            Self::Codex => "codex-responses",
            Self::Claude => "claude",
            Self::Gemini => "gemini",
            Self::Unsupported => "unsupported",
        }
    }
    fn budget(self) -> Option<&'static str> {
        match self {
            Self::Chat | Self::Claude => Some("/max_tokens"),
            Self::Responses => Some("/max_output_tokens"),
            Self::Gemini => Some("/generationConfig/maxOutputTokens"),
            _ => None,
        }
    }
    fn temperature(self) -> Option<&'static str> {
        match self {
            Self::Chat | Self::Responses | Self::Claude => Some("/temperature"),
            Self::Gemini => Some("/generationConfig/temperature"),
            _ => None,
        }
    }
    fn top_p(self) -> Option<&'static str> {
        match self {
            Self::Chat | Self::Responses | Self::Claude => Some("/top_p"),
            Self::Gemini => Some("/generationConfig/topP"),
            _ => None,
        }
    }
    fn stop(self) -> Option<&'static str> {
        match self {
            Self::Chat => Some("/stop"),
            Self::Claude => Some("/stop_sequences"),
            Self::Gemini => Some("/generationConfig/stopSequences"),
            _ => None,
        }
    }
    fn system(self) -> Option<&'static str> {
        match self {
            Self::Chat => Some("/messages"),
            Self::Responses | Self::Codex => Some("/instructions"),
            Self::Claude => Some("/system"),
            Self::Gemini => Some("/systemInstruction"),
            Self::Unsupported => None,
        }
    }
}
fn service(provider: &Provider) -> bool {
    crate::plugins::channel_plugin_id(provider).is_some()
}
pub(super) fn profile(provider: &Provider, path: &str) -> Profile {
    if service(provider) {
        return match path {
            "/v1/chat/completions" => Profile::Chat,
            "/v1/responses" => Profile::Responses,
            _ => Profile::Unsupported,
        };
    }
    let engine = provider.engine.trim().to_ascii_lowercase();
    if provider
        .preferences
        .get("protocol_mode")
        .and_then(Value::as_str)
        == Some("native")
        && ["gpt", "openrouter"].contains(&engine.as_str())
    {
        return match path {
            "/v1/chat/completions" => Profile::Chat,
            "/v1/responses" | "/v1/responses/compact" => Profile::Responses,
            _ => Profile::Unsupported,
        };
    }
    if !matches!(
        path,
        "/v1/chat/completions" | "/v1/messages" | "/v1/responses" | "/v1/responses/compact"
    ) && crate::protocols::dialects::gemini_route(path).is_none()
    {
        return Profile::Unsupported;
    }
    if engine == "codex" {
        return Profile::Codex;
    }
    if path == "/v1/responses/compact" {
        return Profile::Unsupported;
    }
    match engine.as_str() {
        "gemini" | "vertex" | "vertex-gemini" => Profile::Gemini,
        "claude" | "vertex-claude" | "aws" => Profile::Claude,
        "gpt" | "openrouter" | "azure" | "azure-databricks"
            if provider
                .base_url
                .to_ascii_lowercase()
                .contains("/responses") =>
        {
            Profile::Responses
        }
        "gpt" | "openrouter" | "azure" | "azure-databricks" => Profile::Chat,
        _ => Profile::Unsupported,
    }
}
fn scope(provider: &Provider) -> &'static str {
    if service(provider) {
        "service_request_body"
    } else {
        "supplier_wire"
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct PolicyTrace {
    pub(crate) path: String,
    pub(crate) source: String,
    pub(crate) action: String,
}
#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct PolicyReport {
    pub(crate) configured: bool,
    pub(crate) profile: String,
    pub(crate) scope: String,
    pub(crate) body_supported: bool,
    pub(crate) header_supported: bool,
    pub(crate) trace: Vec<PolicyTrace>,
    pub(crate) warnings: Vec<String>,
    pub(crate) sensitive_headers: Vec<String>,
}
impl PolicyReport {
    fn new(provider: &Provider, path: &str) -> Self {
        let profile = profile(provider, path);
        Self {
            configured: provider.preferences.contains_key("parameter_policy"),
            profile: profile.name().into(),
            scope: scope(provider).into(),
            body_supported: profile != Profile::Unsupported,
            header_supported: !service(provider),
            ..Default::default()
        }
    }
    pub(crate) fn record(&mut self, path: &str, source: &str, action: &str) {
        self.trace.push(PolicyTrace {
            path: path.into(),
            source: source.into(),
            action: action.into(),
        });
    }
}

pub(crate) fn header_sensitive(name: &str, declared_secret: bool) -> bool {
    let name = name.to_ascii_lowercase().replace('_', "-");
    declared_secret
        || [
            "auth",
            "cookie",
            "api-key",
            "apikey",
            "token",
            "secret",
            "access-key",
            "private-key",
        ]
        .iter()
        .any(|part| name.contains(part))
}

pub(crate) fn capabilities(provider: &Provider, path: &str) -> Value {
    let profile = profile(provider, path);
    let supported = profile != Profile::Unsupported;
    let nullable = matches!(profile, Profile::Chat | Profile::Responses);
    let field = |path: Option<&str>,
                 kind: &str,
                 minimum: Option<f64>,
                 maximum: Option<f64>,
                 nullable: bool| {
        json!({"supported":path.is_some(),"path":path,"type":kind,"minimum":minimum,"maximum":maximum,"nullable":nullable})
    };
    let generation = json!({
        "temperature":field(profile.temperature(), "number", Some(0.0), Some(if profile==Profile::Claude {1.0}else{2.0}), nullable),
        "top_p":field(profile.top_p(), "number", Some(0.0), Some(1.0), nullable),
        "output_limit":field(profile.budget(), "integer", Some(1.0), None, false),
        "stop":field(profile.stop(), if profile==Profile::Chat {"string_or_array"}else{"array"}, None, None, profile==Profile::Chat)
    });
    let fields = generation
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, value)| {
            let mut value = value.clone();
            value
                .as_object_mut()
                .unwrap()
                .insert("name".into(), json!(name));
            value
        })
        .collect::<Vec<_>>();
    let mut endpoints = Vec::new();
    if !service(provider) {
        for endpoint in [
            "/v1/chat/completions",
            "/v1/responses",
            "/v1/messages",
            "/v1/responses/compact",
        ] {
            if self::profile(provider, endpoint) != Profile::Unsupported {
                endpoints.push(endpoint);
            }
        }
    }
    json!({"version":1,"endpoint":path,"endpoints":endpoints,"profile":profile.name(),"scope":scope(provider),"body_supported":supported,"header_supported":!service(provider),"native_preview":native_responses_candidate(provider,path),"preview_supported":supported,"plugin_envelope":service(provider),"generation":generation,"fields":fields,"system_prompt":{"supported":supported,"path":profile.system(),"modes":["inherit","prepend","append","replace","remove"]},"unsupported":["reasoning_effort","thinking","service_tier"],"remove":{"supported":supported,"format":"json_pointer"},"note":"Protocol shape capability only; model-specific availability is not asserted. Service endpoint choices must be intersected with its registered manifest."})
}

pub(crate) fn native_responses_candidate(provider: &Provider, endpoint: &str) -> bool {
    !service(provider)
        && matches!(endpoint, "/v1/responses" | "/v1/responses/compact")
        // Match the registered native executor gate exactly. Uppercase/alias
        // engines currently use the generic production path; preview must too.
        && (provider.engine.as_ref() == "codex"
            || (provider.engine.as_ref() == "gpt"
                && provider.base_url.contains("v1/responses")))
}

/// No scheduling, OAuth refresh, key rotation, external fetch or network send.
/// Native Responses uses exactly its production payload/header builders. Other
/// paths use the registry's ordinary production prepare, including service v1.
pub(crate) fn prepare_preview(
    registry: &crate::plugins::PluginRegistry,
    provider: &Provider,
    requested_model: &str,
    upstream_model: &str,
    endpoint: &str,
    payload: Value,
) -> Result<PreparedAttempt, String> {
    let uri = endpoint
        .parse()
        .map_err(|_| "preview endpoint is invalid".to_owned())?;
    let key = provider
        .api_keys
        .first()
        .map(String::as_str)
        .unwrap_or(if service(provider) {
            "__channel_plugin__"
        } else {
            "preview-key"
        });
    let key = if key.contains(',') && provider.engine.eq_ignore_ascii_case("codex") {
        "oauth-resolved-at-dispatch"
    } else {
        key
    };
    if native_responses_candidate(provider, endpoint) {
        let compact = endpoint == "/v1/responses/compact";
        let stream_value = payload.get("stream").cloned();
        let stream = match stream_value.as_ref() {
            None | Some(Value::Null) => false,
            Some(value) => crate::upstream::responses::prepare::pydantic_bool(value)
                .ok_or("preview stream must be a boolean-compatible value")?,
        };
        let mut body = payload;
        if stream_value.is_some_and(|value| !matches!(value, Value::Null | Value::Bool(_))) {
            body.as_object_mut()
                .ok_or("preview body must be an object")?
                .insert("stream".into(), Value::Bool(stream));
        }
        let engine = provider.engine.to_ascii_lowercase();
        let mut report = crate::upstream::responses::route::compile_payload(
            &mut body,
            provider,
            requested_model,
            upstream_model,
            &engine,
            compact,
        )?;
        let mut headers = crate::upstream::responses::route::build_headers(
            &HeaderMap::new(),
            provider,
            key,
            &engine,
            stream,
            "preview",
            "preview-attempt",
        )?;
        apply_native_headers(provider, endpoint, &mut headers, &mut report)?;
        registry.native_headers(&engine, &mut headers)?;
        report.warnings.push(
            "Preview does not resolve or refresh OAuth; dispatch-time authentication may differ."
                .into(),
        );
        let mut typed = HeaderMap::new();
        for (name, value) in headers {
            typed.insert(
                name.parse::<axum::http::HeaderName>()
                    .map_err(|_| "invalid preview header")?,
                value.parse().map_err(|_| "invalid preview header value")?,
            );
        }
        return Ok(PreparedAttempt {
            policy_report: report,
            plugin_lease: None,
            dispatch: None,
            method: Method::POST,
            url: crate::upstream::responses::route::normalize_upstream_url(
                &provider.base_url,
                &engine,
                compact,
            ),
            headers: typed,
            body: AttemptBody::Json(serde_json::to_vec(&body).map_err(|_| "encode preview body")?),
            adapter: ResponseAdapter::Passthrough,
            downstream_stream: stream,
            upstream_stream: stream,
            request_model: requested_model.into(),
            original_model: upstream_model.into(),
            wire_model: body.get("model").and_then(Value::as_str).map(str::to_owned),
            downstream_protocol: DownstreamProtocol::Native,
            chat_stream_include_usage: false,
            provider_key: key.into(),
            estimated_video_tokens: None,
        });
    }
    let input = PreparedInput {
        payload: Some(payload),
        replay: None,
        observation: Default::default(),
        default_model: String::new(),
        content_type: "application/json".into(),
    };
    registry.prepare(
        provider,
        key,
        requested_model,
        upstream_model,
        &Method::POST,
        &uri,
        endpoint,
        &HeaderMap::new(),
        &input,
        "preview",
    )
}

/// Add a value-free trace of final protocol sanitizers after policy application.
pub(crate) fn record_constraints(before: &Value, after: &Value, report: &mut PolicyReport) {
    fn walk(before: Option<&Value>, after: Option<&Value>, path: &str, report: &mut PolicyReport) {
        if before == after {
            return;
        }
        if let (Some(Value::Object(a)), Some(Value::Object(b))) = (before, after) {
            let keys: std::collections::BTreeSet<_> = a.keys().chain(b.keys()).collect();
            for key in keys {
                walk(
                    a.get(key),
                    b.get(key),
                    &format!("{path}/{}", key.replace('~', "~0").replace('/', "~1")),
                    report,
                );
            }
        } else {
            report.record(
                path,
                "protocol_constraint",
                if after.is_some() { "set" } else { "remove" },
            );
        }
    }
    walk(Some(before), Some(after), "", report);
}
