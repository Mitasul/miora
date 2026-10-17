use super::manifest::Manifest;
use crate::config::snapshot::Provider;
use crate::providers::types::PreparedInput;
use crate::runtime::context::AppState;
use crate::transport::body::{read_spooled_body, RequestBodySpoolError};
use crate::transport::http::{is_hop_by_hop, json_error};
use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderMap, HeaderValue, Method, Response, StatusCode, Uri};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

pub(crate) use super::native_stream::relay;

#[cfg(test)]
mod tests;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeProxy {
    pub client_auth: ClientAuth,
    pub routes: Vec<NativeRoute>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ClientAuth {
    Bearer,
    Header { name: String },
    Query { name: String },
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeRoute {
    pub path: String,
    pub methods: Vec<String>,
    pub model: NativeModel,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum NativeModel {
    Body { pointer: String },
    Form { field: String },
    Query { name: String },
    Path,
    Fixed { value: String },
}

fn decode_component(value: &str) -> String {
    url::form_urlencoded::parse(format!("value={}", value.replace('+', "%2B")).as_bytes())
        .next()
        .map(|(_, value)| value.into_owned())
        .unwrap_or_default()
}

pub(crate) fn mount(path: &str) -> Option<(String, &str)> {
    let tail = path.strip_prefix("/native/")?;
    let (channel, _) = tail.split_once('/')?;
    if channel.is_empty() {
        return None;
    }
    Some((decode_component(channel), &tail[channel.len()..]))
}

impl NativeRoute {
    pub fn path_model<'a>(&self, path: &'a str) -> Option<&'a str> {
        let (prefix, suffix) = self.path.split_once("{model}")?;
        let model = path.strip_prefix(prefix)?.strip_suffix(suffix)?;
        (!model.is_empty() && !model.contains('/')).then_some(model)
    }

    fn matches(&self, method: &Method, path: &str) -> bool {
        self.methods
            .iter()
            .any(|allowed| allowed == method.as_str())
            && (self.path == path
                || self
                    .path
                    .strip_suffix('*')
                    .is_some_and(|prefix| path.starts_with(prefix))
                || self.path_model(path).is_some())
    }
}

impl NativeProxy {
    pub fn validate(&self) -> Result<(), String> {
        match &self.client_auth {
            ClientAuth::Header { name } if name.parse::<axum::http::HeaderName>().is_err() => {
                return Err("invalid native client credential header".into());
            }
            ClientAuth::Query { name } if name.is_empty() => {
                return Err("native client credential query name is required".into());
            }
            _ => {}
        }
        if self.routes.is_empty() {
            return Err("native proxy routes are required".into());
        }
        for route in &self.routes {
            if !route.path.starts_with('/')
                || route.path.starts_with("//")
                || route.path.contains(['?', '#', '\\'])
                || route.path.matches('*').count() > usize::from(route.path.ends_with('*'))
                || route.path.matches("{model}").count() > 1
                || route.methods.is_empty()
                || route.methods.iter().any(|method| {
                    !matches!(
                        method.as_str(),
                        "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE"
                    )
                })
            {
                return Err("invalid native route path or method".into());
            }
            match &route.model {
                NativeModel::Body { pointer } if !pointer.starts_with('/') => {
                    return Err("native body model requires a JSON pointer".into())
                }
                NativeModel::Form { field } if field.is_empty() => {
                    return Err("native form model field is required".into())
                }
                NativeModel::Query { name } if name.is_empty() => {
                    return Err("native model query name is required".into())
                }
                NativeModel::Path if !route.path.contains("{model}") => {
                    return Err("native path model requires {model}".into())
                }
                NativeModel::Fixed { value } if value.is_empty() => {
                    return Err("native fixed authorization model is required".into())
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub fn route(&self, method: &Method, path: &str) -> Option<&NativeRoute> {
        self.routes.iter().find(|route| route.matches(method, path))
    }

    pub fn query(&self, query: Option<&str>, manifest: &Manifest) -> String {
        query
            .unwrap_or("")
            .split('&')
            .filter(|part| {
                let name = part.split_once('=').map_or(*part, |(name, _)| name);
                let name = decode_component(name);
                !(matches!(&self.client_auth, ClientAuth::Query { name: credential } if credential == &name)
                    || manifest.auth.kind == "query" && manifest.auth.query == name)
            })
            .collect::<Vec<_>>()
            .join("&")
    }

    pub fn headers(&self, incoming: &HeaderMap, manifest: &Manifest) -> Vec<(String, String)> {
        let connection = incoming
            .get_all("connection")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(','))
            .map(str::trim)
            .collect::<Vec<_>>();
        incoming.iter().filter_map(|(name, value)| {
            let name = name.as_str();
            if is_hop_by_hop(name)
                || connection.iter().any(|hop| hop.eq_ignore_ascii_case(name))
                || matches!(name, "host" | "content-length" | "authorization" | "x-api-key" | "x-goog-api-key" | "cookie" | "forwarded")
                || name.starts_with("x-miora-") || name.starts_with("x-uni-api-") || name.starts_with("x-forwarded-")
                || matches!(&self.client_auth, ClientAuth::Header { name: credential } if credential.eq_ignore_ascii_case(name))
                || (manifest.auth.kind == "api_key" && manifest.auth.header.eq_ignore_ascii_case(name))
            {
                None
            } else {
                Some((name.to_owned(), value.as_bytes().iter().map(|byte| char::from(*byte)).collect()))
            }
        }).collect()
    }
}

pub(crate) fn channel(
    state: &AppState,
    providers: &std::collections::HashMap<String, Arc<Provider>>,
    path: &str,
) -> Result<(Arc<Provider>, Arc<super::registry::Version>), Response<Body>> {
    let (name, _) =
        mount(path).ok_or_else(|| json_error(StatusCode::NOT_FOUND, "Native channel not found"))?;
    let provider = providers
        .get(&name)
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "Native channel not found"))?;
    let plugins = state.plugins.snapshot();
    let version = super::channel_plugin_id(provider)
        .and_then(|id| plugins.versions.get(id))
        .filter(|version| version.manifest.native_proxy.is_some())
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "Native plugin is unavailable"))?;
    Ok((provider.clone(), version.clone()))
}

pub(crate) async fn normalize_request(
    state: &AppState,
    request: &mut Request,
) -> Result<(), Response<Body>> {
    if mount(request.uri().path()).is_none() {
        return Ok(());
    }
    let snapshot = state.runtime.snapshot().await.ok_or_else(|| {
        json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Runtime configuration is not ready",
        )
    })?;
    let (_, version) = channel(state, &snapshot.providers_by_name, request.uri().path())?;
    normalize_credentials(&version.manifest, request)
}

fn normalize_credentials(manifest: &Manifest, request: &mut Request) -> Result<(), Response<Body>> {
    let native = manifest.native_proxy.as_ref().unwrap();
    let credential = match &native.client_auth {
        ClientAuth::Bearer => request
            .headers()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split_once(' '))
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
            .map(|(_, token)| token.trim().to_owned()),
        ClientAuth::Header { name } => request
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned),
        ClientAuth::Query { name } => crate::upstream::generic::query_value(request.uri(), name),
    }
    .filter(|value| !value.is_empty())
    .ok_or_else(|| json_error(StatusCode::FORBIDDEN, "Invalid or missing API Key"))?;
    let credential = HeaderValue::from_str(&credential)
        .map_err(|_| json_error(StatusCode::BAD_REQUEST, "Invalid API key header"))?;
    request.headers_mut().insert("x-api-key", credential);
    for name in [
        "x-miora-byok-provider",
        "x-miora-byok-api-key",
        crate::routing::planner::TARGET_PROVIDER_HEADER,
        crate::routing::planner::PROBE_UNCONFIGURED_MODEL_HEADER,
    ] {
        request.headers_mut().remove(name);
    }
    let query = native.query(request.uri().query(), manifest);
    let target = if query.is_empty() {
        request.uri().path().to_owned()
    } else {
        format!("{}?{query}", request.uri().path())
    };
    *request.uri_mut() = target
        .parse()
        .map_err(|_| json_error(StatusCode::BAD_REQUEST, "Invalid native request URI"))?;
    Ok(())
}

pub(crate) async fn prepare_input(
    state: &AppState,
    request: Request,
    method: &Method,
    uri: &Uri,
    resource_wait: Duration,
) -> Result<PreparedInput, Response<Body>> {
    let snapshot = state.runtime.snapshot().await.ok_or_else(|| {
        json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Runtime configuration is not ready",
        )
    })?;
    let (provider, version) = channel(state, &snapshot.providers_by_name, uri.path())?;
    let (_, path) = mount(uri.path()).unwrap();
    let route = version
        .manifest
        .native_proxy
        .as_ref()
        .unwrap()
        .route(method, path)
        .ok_or_else(|| {
            json_error(
                StatusCode::NOT_FOUND,
                "Native route is not declared by this plugin",
            )
        })?;
    let auth = state
        .runtime
        .authorize(request.headers())
        .await
        .map_err(|error| json_error(error.status, &error.message))?;
    let content_length = request
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok());
    let content_type = request
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_owned();
    let metadata_headers = request.headers().clone();
    let spool = read_spooled_body(
        request.into_body(),
        &state.request_spool,
        None,
        content_length,
        resource_wait,
    )
    .await
    .map_err(|error| match error {
        RequestBodySpoolError::Timeout => {
            json_error(StatusCode::REQUEST_TIMEOUT, "Request body upload timed out")
        }
        RequestBodySpoolError::Read => {
            json_error(StatusCode::BAD_REQUEST, "Request body upload failed")
        }
        RequestBodySpoolError::Spool(failure) => json_error(failure.status, &failure.message),
    })?;
    let mut stream = false;
    let upstream_model = match &route.model {
        NativeModel::Body { pointer } => {
            let payload = if metadata_headers.contains_key("content-encoding") {
                let body = spool
                    .storage
                    .clone_for_replay()
                    .into_body(&spool.observation)
                    .await
                    .map_err(|failure| json_error(failure.status, &failure.message))?;
                let mut metadata = Request::new(body);
                *metadata.headers_mut() = metadata_headers;
                let metadata = crate::transport::decompression::decode(metadata).await?;
                let bytes = axum::body::to_bytes(
                    metadata.into_body(),
                    crate::transport::decompression::RequestBodyLimits::from_env().max_bytes,
                )
                .await
                .map_err(|_| {
                    json_error(StatusCode::BAD_REQUEST, "Cannot read native model metadata")
                })?;
                serde_json::from_slice::<Value>(&bytes)
                    .map_err(|error| json_error(StatusCode::BAD_REQUEST, &error.to_string()))?
            } else {
                spool
                    .storage
                    .parse_json()
                    .await
                    .map_err(|error| json_error(StatusCode::BAD_REQUEST, &error))?
            };
            stream = payload
                .get("stream")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            payload
                .pointer(pointer)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned()
        }
        NativeModel::Form { field } => {
            if content_type
                .split(';')
                .next()
                .unwrap_or("")
                .eq_ignore_ascii_case("multipart/form-data")
            {
                spool
                    .storage
                    .multipart_text_field(&content_type, field, 4096)
                    .await
                    .map_err(|error| json_error(StatusCode::BAD_REQUEST, &error))?
                    .unwrap_or_default()
            } else {
                spool
                    .storage
                    .urlencoded_text_fields(&[field])
                    .await
                    .map_err(|error| json_error(StatusCode::BAD_REQUEST, &error))?
                    .get(field)
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned()
            }
        }
        NativeModel::Query { name } => {
            crate::upstream::generic::query_value(uri, name).unwrap_or_default()
        }
        NativeModel::Path => decode_component(route.path_model(path).unwrap()),
        NativeModel::Fixed { value } => value.clone(),
    };
    let model = provider
        .models
        .iter()
        .filter(|(_, original)| **original == upstream_model)
        .map(|(alias, _)| alias)
        .find(|alias| {
            crate::routing::catalog::model_entries(&snapshot, &auth.api_key, alias)
                .iter()
                .any(|(candidate, _)| candidate.name == provider.name)
        })
        .cloned()
        .ok_or_else(|| {
            json_error(
                StatusCode::FORBIDDEN,
                "Native model is not allowed for this API key and channel",
            )
        })?;
    Ok(PreparedInput {
        payload: Some(json!({"model":model,"stream":stream})),
        replay: Some((spool.storage, spool.observation.clone())),
        observation: spool.observation,
        default_model: model,
        content_type,
    })
}
