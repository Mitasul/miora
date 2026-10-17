use super::lease::PluginLease;
use super::registry::{PluginRegistry, Version};
use crate::config::snapshot::Provider;
use crate::providers::types::{
    AttemptBody, DownstreamProtocol, PreparedAttempt, PreparedInput, ResponseAdapter,
};
use axum::http::{HeaderMap, HeaderValue, Method, Uri};
use futures_util::StreamExt;
use serde_json::{json, Map, Value};

pub(crate) fn channel_plugin_id(provider: &Provider) -> Option<&str> {
    provider
        .preferences
        .get("channel_plugin")
        .and_then(Value::as_str)
        .or_else(|| provider.engine.strip_prefix("plugin:"))
}
pub(super) fn service_headers(version: &Version) -> Result<HeaderMap, String> {
    if version.managed.is_some() && version.runtime.is_none() {
        return Err(
            "Managed plugin is not running; inspect its error and logs before enabling it".into(),
        );
    }
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    headers.insert(
        "x-miora-plugin-version",
        HeaderValue::from_str(&version.manifest.version).map_err(|_| "invalid plugin version")?,
    );
    if version.service_auth.kind == "bearer" {
        headers.insert(
            "authorization",
            HeaderValue::from_str(&format!("Bearer {}", version.service_auth.token))
                .map_err(|_| "invalid service credential".to_owned())?,
        );
    }
    Ok(headers)
}
impl PluginRegistry {
    pub fn validate_channel_config(
        &self,
        id: &str,
        config: &Map<String, Value>,
    ) -> Result<(), String> {
        let snapshot = self.snapshot();
        let version = snapshot
            .versions
            .get(id)
            .ok_or("channel plugin is missing or disabled")?;
        let mut merged = version.config.clone();
        merged.extend(config.clone());
        version.manifest.config(&merged, false).map(|_| ())
    }
    pub fn native_responses_allowed(&self, provider: &Provider) -> bool {
        channel_plugin_id(provider).is_none()
            && super::builtin::registry()
                .get(provider.engine.as_ref())
                .is_some_and(|p| ["gpt", "codex"].contains(&p.engine))
    }
    pub fn native_headers(
        &self,
        engine: &str,
        headers: &mut std::collections::HashMap<String, String>,
    ) -> Result<(), String> {
        let snapshot = self.snapshot();
        if snapshot.interceptors.is_empty() {
            return Ok(());
        }
        let mut typed = HeaderMap::new();
        for (name, value) in headers.iter() {
            typed.insert(
                name.parse::<axum::http::HeaderName>()
                    .map_err(|_| "invalid native header")?,
                value
                    .parse::<HeaderValue>()
                    .map_err(|_| "invalid native header")?,
            );
        }
        super::interceptors::apply(
            &snapshot.interceptors,
            &format!("builtin-{engine}"),
            &mut typed,
        );
        *headers = typed
            .iter()
            .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_owned())))
            .collect();
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &self,
        provider: &Provider,
        provider_key: &str,
        request_model: &str,
        original_model: &str,
        method: &Method,
        uri: &Uri,
        path: &str,
        incoming_headers: &HeaderMap,
        input: &PreparedInput,
        request_id: &str,
    ) -> Result<PreparedAttempt, String> {
        let snapshot = self.snapshot();
        let Some(id) = channel_plugin_id(provider) else {
            let mut prepared = (super::builtin::resolve(&provider.engine.to_ascii_lowercase())
                .prepare)(
                provider,
                provider_key,
                request_model,
                original_model,
                method,
                uri,
                path,
                incoming_headers,
                input,
                request_id,
            )?;
            let id = format!("builtin-{}", provider.engine.to_ascii_lowercase());
            super::interceptors::apply(&snapshot.interceptors, &id, &mut prepared.headers);
            prepared.plugin_lease = Some(PluginLease {
                snapshot,
                service: false,
            });
            return Ok(prepared);
        };
        let version = snapshot
            .versions
            .get(id)
            .ok_or("channel plugin is missing or disabled")?;
        let native = super::native::mount(uri.path());
        if native.is_none()
            && !version
                .manifest
                .capabilities
                .endpoints
                .iter()
                .any(|e| e == path)
        {
            return Err("channel plugin does not support endpoint".into());
        }
        let media_type = input.content_type.split(';').next().unwrap_or("").trim();
        let format = if input.replay.is_none() {
            "json"
        } else if media_type.eq_ignore_ascii_case("multipart/form-data") {
            "multipart"
        } else if media_type.eq_ignore_ascii_case("application/x-www-form-urlencoded") {
            "form"
        } else {
            "binary"
        };
        if native.is_none()
            && !version
                .manifest
                .capabilities
                .request_formats
                .iter()
                .any(|value| value == format)
        {
            return Err(format!("channel plugin does not accept {format} requests"));
        }
        let stream = input
            .payload
            .as_ref()
            .and_then(|p| p.get("stream"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if native.is_none() && stream && !version.manifest.capabilities.streaming {
            return Err("channel plugin does not support streaming".into());
        }
        let mut config = version.config.clone();
        if let Some(overrides) = provider.preferences.get("plugin_config") {
            let overrides = overrides
                .as_object()
                .ok_or("plugin_config must be an object")?;
            config.extend(overrides.clone());
        }
        // Scheduled channel key is used only for the declared secret field. The
        // gateway caller credential, cookies and admin session NEVER cross this boundary.
        if !version.manifest.auth.config_key.is_empty()
            && !provider_key.is_empty()
            && provider_key != "__channel_plugin__"
        {
            config.insert(
                version.manifest.auth.config_key.clone(),
                Value::String(provider_key.to_owned()),
            );
        }
        let config = version.manifest.config(&config, true)?;
        if let Some((channel, endpoint)) = native {
            if channel != provider.name.as_ref() {
                return Err("native channel does not match selected provider".into());
            }
            let native = version
                .manifest
                .native_proxy
                .as_ref()
                .ok_or("plugin has no native proxy")?;
            if native.route(method, endpoint).is_none() {
                return Err("native route is not declared by this plugin".into());
            }
            let (storage, observation) = input
                .replay
                .as_ref()
                .ok_or("native body resource is missing")?;
            let envelope = serde_json::to_vec(&json!({
                "api_version":1,
                "plugin":{"id":id,"version":version.manifest.version,"revision":version.revision},
                "request":{"id":request_id,"format":"native","method":method.as_str(),"endpoint":endpoint,
                    "query":native.query(uri.query(), &version.manifest),
                    "headers":native.headers(incoming_headers, &version.manifest),
                    "model":original_model,"requested_model":request_model,"stream":stream,
                    "body_resource":{"resource_id":"body","content_type":input.content_type,"size":observation.body_bytes}},
                "channel":{"name":provider.name.as_ref(),"config":config},"authentication":version.manifest.auth
            })).map_err(|_| "encode native plugin request".to_owned())?;
            let mut headers = service_headers(version)?;
            headers.insert(
                "content-type",
                HeaderValue::from_static("application/vnd.miora.resource+octet-stream"),
            );
            headers.insert(
                "x-miora-envelope-length",
                HeaderValue::from(envelope.len() as u64),
            );
            headers.insert(
                "content-length",
                HeaderValue::from(envelope.len() as u64 + observation.body_bytes),
            );
            headers.insert(
                "x-miora-plugin-revision",
                HeaderValue::from(version.revision),
            );
            super::interceptors::apply(&snapshot.interceptors, id, &mut headers);
            return Ok(PreparedAttempt {
                policy_report: Default::default(),
                plugin_lease: Some(PluginLease {
                    snapshot: snapshot.clone(),
                    service: true,
                }),
                dispatch: None,
                method: Method::POST,
                url: format!("{}/execute", version.endpoint),
                headers,
                body: AttemptBody::PluginResource {
                    envelope,
                    storage: storage.clone_for_replay(),
                    observation: observation.clone(),
                },
                adapter: ResponseAdapter::Passthrough,
                downstream_stream: stream,
                upstream_stream: true,
                request_model: request_model.to_owned(),
                original_model: original_model.to_owned(),
                wire_model: Some(original_model.to_owned()),
                downstream_protocol: DownstreamProtocol::NativeProxy,
                chat_stream_include_usage: false,
                provider_key: provider_key.to_owned(),
                estimated_video_tokens: None,
            });
        }
        let mut payload = input.payload.clone().unwrap_or_else(|| json!({}));
        if let Some(payload) = payload.as_object_mut() {
            payload.insert("model".into(), Value::String(original_model.to_owned()));
        }
        let private_parameters: Map<String, Value> = version
            .manifest
            .request_schema
            .iter()
            .filter_map(|field| {
                payload
                    .as_object_mut()
                    .and_then(|root| root.remove(&field.key))
                    .map(|value| (field.key.clone(), value))
            })
            .collect();
        let mut policy_origin = input
            .replay
            .is_none()
            .then(|| {
                crate::providers::parameters::capture_origin(
                    provider,
                    input.payload.as_ref().unwrap_or(&Value::Null),
                    &payload,
                )
            })
            .flatten();
        if let Some(origin) = policy_origin.as_mut() {
            crate::providers::parameters::exclude_plugin_fields(origin, private_parameters.keys());
        }
        let mut policy_report = crate::providers::parameters::apply_body(
            provider,
            request_model,
            path,
            policy_origin.as_ref(),
            &mut payload,
        )?;
        if let Some(root) = payload.as_object_mut() {
            for key in private_parameters.keys() {
                policy_report.record(
                    &format!("/{}", key.replace('~', "~0").replace('/', "~1")),
                    "plugin_schema",
                    "retain",
                );
            }
            root.extend(private_parameters);
        }
        let resource = input.replay.as_ref().map(|(_, observation)| json!({"resource_id":"body","content_type":input.content_type,"size":observation.body_bytes}));
        let mut envelope = json!({"api_version":1,"plugin":{"id":id,"version":version.manifest.version,"revision":version.revision},"request":{"id":request_id,"method":method.as_str(),"endpoint":path,"model":original_model,"requested_model":request_model,"stream":stream,"body":payload},"channel":{"name":provider.name.as_ref(),"config":config},"authentication":version.manifest.auth});
        let mut query_parameters = Map::new();
        for (name, value) in url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes())
        {
            if !version
                .manifest
                .request_schema
                .iter()
                .any(|field| field.key == name)
            {
                continue;
            }
            match query_parameters.entry(name.into_owned()) {
                serde_json::map::Entry::Vacant(entry) => {
                    entry.insert(Value::String(value.into_owned()));
                }
                serde_json::map::Entry::Occupied(mut entry) => {
                    let values = entry.get_mut();
                    if let Some(values) = values.as_array_mut() {
                        values.push(Value::String(value.into_owned()));
                    } else {
                        *values = json!([values.take(), value]);
                    }
                }
            }
        }
        if !query_parameters.is_empty() {
            envelope["request"]["query_parameters"] = Value::Object(query_parameters);
        }
        if let Some(resource) = resource {
            envelope["request"]["format"] = json!(format);
            envelope["request"]["body_resource"] = resource;
        }
        let body = serde_json::to_vec(&envelope).map_err(|_| "encode plugin request".to_owned())?;
        let mut headers = service_headers(version)?;
        let body = if let Some((storage, observation)) = &input.replay {
            headers.insert(
                "content-type",
                HeaderValue::from_static("application/vnd.miora.resource+octet-stream"),
            );
            headers.insert(
                "x-miora-envelope-length",
                HeaderValue::from(body.len() as u64),
            );
            headers.insert(
                "content-length",
                HeaderValue::from(body.len() as u64 + observation.body_bytes),
            );
            AttemptBody::PluginResource {
                envelope: body,
                storage: storage.clone_for_replay(),
                observation: observation.clone(),
            }
        } else {
            AttemptBody::Json(body)
        };
        if stream {
            headers.insert("accept", HeaderValue::from_static("text/event-stream"));
        }
        headers.insert(
            "x-miora-plugin-revision",
            HeaderValue::from_str(&version.revision.to_string())
                .map_err(|_| "invalid revision".to_owned())?,
        );
        super::interceptors::apply(&snapshot.interceptors, id, &mut headers);
        Ok(PreparedAttempt {
            policy_report,
            plugin_lease: Some(PluginLease {
                snapshot: snapshot.clone(),
                service: true,
            }),
            dispatch: None,
            method: Method::POST,
            url: format!("{}/execute", version.endpoint),
            headers,
            body,
            adapter: ResponseAdapter::Passthrough,
            downstream_stream: stream,
            upstream_stream: stream,
            request_model: request_model.to_owned(),
            original_model: original_model.to_owned(),
            wire_model: Some(original_model.to_owned()),
            downstream_protocol: DownstreamProtocol::Native,
            chat_stream_include_usage: input
                .payload
                .as_ref()
                .and_then(|p| p.pointer("/stream_options/include_usage"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
            provider_key: provider_key.to_owned(),
            estimated_video_tokens: None,
        })
    }
    pub async fn models(
        &self,
        id: &str,
        channel: &str,
        overrides: Map<String, Value>,
    ) -> Result<Value, String> {
        let snapshot = self.snapshot();
        let version = snapshot
            .versions
            .get(id)
            .ok_or("channel plugin is missing or disabled")?;
        if !version.manifest.capabilities.model_discovery {
            return Ok(json!({"models":version.manifest.models}));
        }
        let mut config = version.config.clone();
        for (key, value) in overrides {
            let secret = version
                .manifest
                .config_schema
                .iter()
                .any(|field| field.key == key && field.secret);
            if secret && value.as_str() == Some("***") {
                if !config.contains_key(&key) {
                    return Err("missing previous secret".into());
                }
            } else {
                config.insert(key, value);
            }
        }
        let config = version.manifest.config(&config, true)?;
        let mut headers = service_headers(version)?;
        super::interceptors::apply(&snapshot.interceptors, id, &mut headers);
        let response = self.client.post(format!("{}/models", version.endpoint)).headers(headers)
            .timeout(std::time::Duration::from_secs(15))
            .json(&json!({"api_version":1,"plugin":{"id":id,"version":version.manifest.version,"revision":version.revision},"channel":{"name":channel,"config":config},"authentication":version.manifest.auth})).send().await.map_err(|_| "plugin model discovery transport failure".to_owned())?;
        if !response.status().is_success() {
            return Err(format!(
                "plugin model discovery HTTP {}",
                response.status().as_u16()
            ));
        }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| "plugin model discovery read failure".to_owned())?;
            if bytes.len() + chunk.len() > 1024 * 1024 {
                return Err("plugin model discovery response exceeds limit".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let result: Value = serde_json::from_slice(&bytes)
            .map_err(|_| "invalid plugin model discovery JSON".to_owned())?;
        let models: Vec<super::manifest::Model> = serde_json::from_value(
            result
                .get("models")
                .cloned()
                .ok_or("missing models array")?,
        )
        .map_err(|_| "invalid plugin models".to_owned())?;
        if models.len() > 4096
            || models
                .iter()
                .any(|m| m.id.is_empty() || m.id.len() > 256 || m.name.len() > 256)
        {
            return Err("invalid plugin model list".into());
        }
        Ok(json!({"models":models}))
    }

    pub async fn oauth(
        &self,
        id: &str,
        channel: &str,
        overrides: Map<String, Value>,
        operation: Value,
    ) -> Result<Value, String> {
        let snapshot = self.snapshot();
        let version = snapshot
            .versions
            .get(id)
            .ok_or("channel plugin is missing or disabled")?;
        if !version.manifest.capabilities.oauth {
            return Err("plugin does not declare OAuth support".into());
        }
        if channel.trim().is_empty() {
            return Err("Enter the channel name before OAuth authorization".into());
        }
        let mut config = version.config.clone();
        for (key, value) in overrides {
            if value != "***" {
                config.insert(key, value);
            }
        }
        let config = version.manifest.config(&config, true)?;
        let mut headers = service_headers(version)?;
        super::interceptors::apply(&snapshot.interceptors, id, &mut headers);
        let response = self.client.post(format!("{}/oauth", version.endpoint)).headers(headers)
            .timeout(std::time::Duration::from_secs(75))
            .json(&json!({"api_version":1,"plugin":{"id":id,"version":version.manifest.version,"revision":version.revision},"channel":{"name":channel,"config":config},"authentication":version.manifest.auth,"oauth":operation}))
            .send().await.map_err(|error| format!("plugin OAuth transport failure: {error}"))?;
        let status = response.status();
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| format!("read plugin OAuth response: {error}"))?;
            if bytes.len() + chunk.len() > 256 * 1024 {
                return Err("plugin OAuth response exceeds limit".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            return Err(format!("plugin OAuth returned HTTP {}", status.as_u16()));
        }
        serde_json::from_slice(&bytes)
            .map_err(|error| format!("invalid plugin OAuth response: {error}"))
    }
}
