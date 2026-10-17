use super::lease::PluginLease;
use super::registry::{PluginRegistry, Version};
use crate::config::snapshot::Provider;
use crate::providers::codex::oauth::{CodexOAuthManager, ResolvedOAuth};
use crate::providers::types::{
    AttemptBody, DownstreamProtocol, PreparedAttempt, PreparedInput, ResponseAdapter,
};
use axum::http::{HeaderMap, HeaderValue, Method, Uri};
use futures_util::StreamExt;
use serde_json::{json, Map, Value};
use std::sync::Arc;

#[derive(Debug)]
pub(crate) enum PrepareError {
    OAuth(String),
    Configuration(String),
}

impl PrepareError {
    pub(crate) fn status(&self) -> axum::http::StatusCode {
        match self {
            Self::OAuth(_) => axum::http::StatusCode::UNAUTHORIZED,
            Self::Configuration(_) => axum::http::StatusCode::BAD_REQUEST,
        }
    }
}

impl std::fmt::Display for PrepareError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OAuth(message) => write!(formatter, "Core OAuth: {message}"),
            Self::Configuration(message) => formatter.write_str(message),
        }
    }
}

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
    pub(super) fn balance_source_config(
        provider: &Provider,
        source: Option<&Version>,
    ) -> Result<Map<String, Value>, String> {
        let Some(source) = source else {
            return Ok(Map::new());
        };
        let mut config = source.config.clone();
        if let Some(overrides) = provider
            .preferences
            .get("plugin_config")
            .and_then(Value::as_object)
        {
            config.extend(overrides.clone());
        }
        source.manifest.config(&config, false)
    }

    pub fn balance_plugin_id(&self, provider: &Provider) -> Result<Option<String>, String> {
        let snapshot = self.snapshot();
        if let Some(id) = provider
            .preferences
            .get("balance_plugin")
            .and_then(Value::as_str)
        {
            return snapshot
                .versions
                .get(id)
                .filter(|version| version.manifest.capabilities.balance_query)
                .map(|_| Some(id.to_owned()))
                .ok_or_else(|| {
                    "Selected balance plugin is missing, disabled or does not declare balance_query"
                        .into()
                });
        }
        let channel = channel_plugin_id(provider).and_then(|id| snapshot.versions.get(id));
        if let Some(version) = channel.filter(|version| version.manifest.capabilities.balance_query)
        {
            return Ok(Some(version.manifest.id.clone()));
        }
        let source = channel.map(AsRef::as_ref);
        let source_config = Self::balance_source_config(provider, source)?;
        let mut matches = snapshot.versions.values().filter(|version| {
            version.manifest.capabilities.balance_query
                && Self::matches_balance_target(&version.manifest, provider, source, &source_config)
        });
        let selected = matches.next().map(|version| version.manifest.id.clone());
        if matches.next().is_some() {
            return Err("Multiple balance plugins match; select one in the channel editor".into());
        }
        Ok(selected)
    }

    pub(super) fn matches_balance_target(
        manifest: &super::manifest::Manifest,
        provider: &Provider,
        source: Option<&Version>,
        source_config: &Map<String, Value>,
    ) -> bool {
        let oauth = source
            .filter(|version| version.manifest.auth.kind == "core_oauth")
            .map(|version| version.manifest.auth.provider.as_str());
        let base = source_config
            .get("base_url")
            .and_then(Value::as_str)
            .unwrap_or(&provider.base_url);
        manifest
            .balance_targets
            .iter()
            .any(|target| target.matches(&provider.engine, oauth, base))
    }

    pub(super) fn balance_configuration(
        manifest: &super::manifest::Manifest,
        provider: &Provider,
        provider_key: &str,
        source: Option<&Version>,
    ) -> Result<(Map<String, Value>, String), String> {
        let id = manifest.id.as_str();
        let mut config = provider
            .preferences
            .get(if channel_plugin_id(provider) == Some(id) {
                "plugin_config"
            } else {
                "balance_plugin_config"
            })
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let source_config = Self::balance_source_config(provider, source)?;
        let base = source_config
            .get("base_url")
            .cloned()
            .or_else(|| source.is_none().then(|| json!(provider.base_url.as_ref())));
        if manifest
            .config_schema
            .iter()
            .any(|field| field.key == "base_url")
        {
            if let Some(base) = base {
                config.entry("base_url".to_owned()).or_insert(base);
            }
        }
        let key = if provider_key == "__channel_plugin__" {
            source
                .and_then(|source| source_config.get(&source.manifest.auth.config_key))
                .and_then(Value::as_str)
                .unwrap_or(provider_key)
        } else {
            provider_key
        };
        Ok((config, key.to_owned()))
    }

    pub(crate) async fn balance_envelope(
        oauth: &CodexOAuthManager,
        provider: &Provider,
        provider_key: &str,
        proxy: Option<&str>,
        version: &Version,
        source: Option<&Version>,
    ) -> Result<Value, PrepareError> {
        let (config, key) =
            Self::balance_configuration(&version.manifest, provider, provider_key, source)
                .map_err(PrepareError::Configuration)?;
        let mut envelope =
            Self::discovery_envelope(oauth, version, &provider.name, config, &key, proxy).await?;
        envelope["request"] = json!({"proxy":proxy});
        Ok(envelope)
    }

    pub async fn balance_authenticated(
        &self,
        oauth: &CodexOAuthManager,
        provider: &Provider,
        provider_key: &str,
        proxy: Option<&str>,
        id: &str,
    ) -> Result<Value, String> {
        let snapshot = self.snapshot();
        let version = snapshot
            .versions
            .get(id)
            .ok_or("Channel plugin is missing or disabled")?;
        if !version.manifest.capabilities.balance_query {
            return Err("Plugin does not provide a balance query".into());
        }
        let source = channel_plugin_id(provider)
            .and_then(|id| snapshot.versions.get(id))
            .map(AsRef::as_ref);
        let envelope =
            match Self::balance_envelope(oauth, provider, provider_key, proxy, version, source)
                .await
            {
                Ok(envelope) => envelope,
                Err(PrepareError::OAuth(_)) => return Ok(json!({"status":"auth_error"})),
                Err(error) => return Err(error.to_string()),
            };
        let mut headers = service_headers(version)?;
        super::interceptors::apply(&snapshot.interceptors, id, &mut headers);
        let response = self
            .client
            .post(format!("{}/balance", version.endpoint))
            .headers(headers)
            .timeout(std::time::Duration::from_secs(12))
            .json(&envelope)
            .send()
            .await
            .map_err(|_| "Plugin balance query transport failure")?;
        let status = response.status();
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| "Plugin balance query read failure")?;
            if bytes.len() + chunk.len() > 262_144 {
                return Err("Plugin balance response exceeds limit".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            let error: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            let code = match error.pointer("/error/code").and_then(Value::as_str) {
                Some(
                    code @ ("access_denied" | "timeout" | "network_error" | "rate_limited"
                    | "upstream_error" | "redirect_blocked" | "invalid_response"
                    | "response_too_large" | "unsupported"),
                ) => code,
                _ => "plugin_error",
            };
            return Ok(json!({"status":code}));
        }
        let balance: crate::providers::balances::AccountBalance =
            serde_json::from_slice(&bytes).map_err(|_| "Invalid plugin balance response")?;
        let mut value = balance.into_value(&format!("plugin:{id}"))?;
        for field in ["account_ref", "account_id"] {
            if let Some(metadata) = envelope["resolved_auth"].get(field) {
                value[field] = metadata.clone();
            }
        }
        Ok(value)
    }

    pub fn core_oauth_provider(&self, id: &str) -> Option<String> {
        self.snapshot()
            .versions
            .get(id)
            .filter(|version| version.manifest.auth.kind == "core_oauth")
            .map(|version| version.manifest.auth.provider.clone())
    }

    pub fn core_oauth_single_execution(
        &self,
        provider: &Provider,
        method: &Method,
        path: &str,
    ) -> bool {
        *method == Method::POST
            && matches!(
                path,
                "/v1/images/generations" | "/v1/images/edits" | "/v1/video/tasks"
            )
            && channel_plugin_id(provider)
                .and_then(|id| self.core_oauth_provider(id))
                .is_some()
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_authenticated(
        &self,
        oauth: &CodexOAuthManager,
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
    ) -> Result<PreparedAttempt, PrepareError> {
        let snapshot = self.snapshot();
        let proxy = provider.preferences.get("proxy").and_then(Value::as_str);
        let mut key = provider_key.to_owned();
        let mut resolved = None;
        let mut account_id = None;
        if let Some(id) = channel_plugin_id(provider) {
            let version = snapshot.versions.get(id).ok_or_else(|| {
                PrepareError::Configuration("channel plugin is missing or disabled".into())
            })?;
            if version.manifest.auth.kind == "core_oauth" {
                let auth = oauth
                    .resolve_core(provider_key, &version.manifest.auth.provider, proxy)
                    .await
                    .map_err(PrepareError::OAuth)?;
                key = auth.access_token.clone();
                resolved = Some(auth);
            }
        } else if (matches!(
            provider.engine.to_ascii_lowercase().as_str(),
            "codex" | "claude" | "antigravity"
        ) && (provider_key.contains(',') || provider_key.starts_with("core-oauth:")))
            || provider_key.trim_start().starts_with('{')
        {
            let auth = oauth
                .resolve(provider_key, proxy)
                .await
                .map_err(PrepareError::OAuth)?;
            key = auth.bearer;
            account_id = auth.account_id;
        }
        let mut prepared = self
            .prepare_snapshot(
                snapshot,
                resolved.as_ref(),
                provider,
                &key,
                request_model,
                original_model,
                method,
                uri,
                path,
                incoming_headers,
                input,
                request_id,
            )
            .map_err(PrepareError::Configuration)?;
        if let Some(account_id) = account_id {
            prepared.headers.insert(
                "chatgpt-account-id",
                HeaderValue::from_str(&account_id)
                    .map_err(|_| PrepareError::OAuth("Invalid Codex account_id header".into()))?,
            );
        }
        Ok(prepared)
    }
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
        self.prepare_snapshot(
            self.snapshot(),
            None,
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
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_snapshot(
        &self,
        snapshot: Arc<super::registry::Snapshot>,
        resolved: Option<&ResolvedOAuth>,
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
            let mut envelope = json!({
                "api_version":1,
                "plugin":{"id":id,"version":version.manifest.version,"revision":version.revision},
                "request":{"id":request_id,"format":"native","method":method.as_str(),"endpoint":endpoint,
                    "query":native.query(uri.query(), &version.manifest),
                    "headers":native.headers(incoming_headers, &version.manifest),
                    "model":original_model,"requested_model":request_model,"stream":stream,
                    "body_resource":{"resource_id":"body","content_type":input.content_type,"size":observation.body_bytes}},
                "channel":{"name":provider.name.as_ref(),"config":config},"authentication":version.manifest.auth
            });
            if let Some(resolved) = resolved {
                envelope["resolved_auth"] = json!(resolved);
            }
            let envelope = serde_json::to_vec(&envelope)
                .map_err(|_| "encode native plugin request".to_owned())?;
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
        if let Some(resolved) = resolved {
            envelope["resolved_auth"] = json!(resolved);
        }
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
    #[cfg(test)]
    pub async fn models(
        &self,
        id: &str,
        channel: &str,
        overrides: Map<String, Value>,
    ) -> Result<Value, String> {
        self.models_authenticated(&CodexOAuthManager::new(), id, channel, overrides, "", None)
            .await
    }

    pub async fn models_authenticated(
        &self,
        oauth: &CodexOAuthManager,
        id: &str,
        channel: &str,
        overrides: Map<String, Value>,
        provider_key: &str,
        proxy: Option<&str>,
    ) -> Result<Value, String> {
        let snapshot = self.snapshot();
        let version = snapshot
            .versions
            .get(id)
            .ok_or("channel plugin is missing or disabled")?;
        if !version.manifest.capabilities.model_discovery {
            return Ok(json!({"models":version.manifest.models}));
        }
        let envelope =
            Self::discovery_envelope(oauth, version, channel, overrides, provider_key, proxy)
                .await
                .map_err(|error| error.to_string())?;
        let mut headers = service_headers(version)?;
        super::interceptors::apply(&snapshot.interceptors, id, &mut headers);
        let response = self
            .client
            .post(format!("{}/models", version.endpoint))
            .headers(headers)
            .timeout(std::time::Duration::from_secs(15))
            .json(&envelope)
            .send()
            .await
            .map_err(|_| "plugin model discovery transport failure".to_owned())?;
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

    pub(crate) async fn discovery_envelope(
        oauth: &CodexOAuthManager,
        version: &Version,
        channel: &str,
        overrides: Map<String, Value>,
        provider_key: &str,
        proxy: Option<&str>,
    ) -> Result<Value, PrepareError> {
        let mut config = version.config.clone();
        for (key, value) in overrides {
            let secret = version
                .manifest
                .config_schema
                .iter()
                .any(|field| field.key == key && field.secret);
            if secret && value.as_str() == Some("***") {
                if !config.contains_key(&key) {
                    return Err(PrepareError::Configuration(
                        "missing previous secret".into(),
                    ));
                }
            } else {
                config.insert(key, value);
            }
        }
        let resolved = if version.manifest.auth.kind == "core_oauth" {
            Some(
                oauth
                    .resolve_core(provider_key, &version.manifest.auth.provider, proxy)
                    .await
                    .map_err(PrepareError::OAuth)?,
            )
        } else {
            if !version.manifest.auth.config_key.is_empty()
                && !provider_key.is_empty()
                && provider_key != "__channel_plugin__"
            {
                config.insert(
                    version.manifest.auth.config_key.clone(),
                    json!(provider_key),
                );
            }
            None
        };
        let config = version
            .manifest
            .config(&config, true)
            .map_err(PrepareError::Configuration)?;
        let mut envelope = json!({"api_version":1,"plugin":{"id":version.manifest.id,"version":version.manifest.version,"revision":version.revision},"channel":{"name":channel,"config":config},"authentication":version.manifest.auth});
        if let Some(resolved) = resolved {
            envelope["resolved_auth"] = json!(resolved);
        }
        Ok(envelope)
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
