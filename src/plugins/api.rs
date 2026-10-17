//! Internal adapter. The caller MUST authenticate an administrator session and
//! verify CSRF before calling this handler; it never accepts a provider API key.
use super::interceptors::HeaderInterceptor;
use super::registry::{PluginRegistry, Registration};
use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, Response, StatusCode};
use serde::Deserialize;
use serde_json::{json, Value};

pub(crate) async fn handle(
    registry: &PluginRegistry,
    request: Request<Body>,
    providers: &[std::sync::Arc<crate::config::snapshot::Provider>],
    oauth: &crate::providers::codex::oauth::CodexOAuthManager,
) -> Response<Body> {
    let path = request.uri().path().trim_end_matches('/').to_owned();
    let method = request.method().clone();
    if path == "/admin/api/plugins/template" && method == Method::GET {
        let mut response = output(StatusCode::OK, super::upload::template());
        response.headers_mut().insert(
            "content-disposition",
            "attachment; filename=example.miora.json".parse().unwrap(),
        );
        return response;
    }
    if path == "/admin/api/plugins/sdk" && method == Method::GET {
        return Response::builder()
            .header("content-type", "text/x-python; charset=utf-8")
            .header(
                "content-disposition",
                "attachment; filename=miora_plugin.py",
            )
            .body(Body::from(include_str!("../../sdk/python/miora_plugin.py")))
            .unwrap();
    }
    if let Some(id) = path
        .strip_prefix("/admin/api/plugins/")
        .and_then(|tail| tail.strip_suffix("/logs"))
    {
        if method == Method::GET {
            return match registry.logs(id) {
                Ok(value) => output(StatusCode::OK, value),
                Err(message) => error(StatusCode::NOT_FOUND, "not_found", &message),
            };
        }
    }
    if matches!(
        path.as_str(),
        "/admin/api/plugins/upload" | "/admin/api/plugins/preview"
    ) && method == Method::POST
    {
        let upload = match super::upload::read(request).await {
            Ok(registration) => registration,
            Err((status, message)) => return error(status, "invalid_plugin_file", &message),
        };
        let preview = path.ends_with("/preview");
        let result = match upload {
            super::upload::Upload::Service(registration) => {
                if preview {
                    return output(
                        StatusCode::OK,
                        json!({"kind":"service","manifest":registration.manifest}),
                    );
                }
                if let Err(message) = registry.validate_instances(
                    &registration.manifest,
                    Some(&registration.config),
                    providers,
                ) {
                    return error(StatusCode::BAD_REQUEST, "incompatible_channel", &message);
                }
                registry.register_checked(registration).await
            }
            super::upload::Upload::Managed { bytes, expected } => {
                let (staging, metadata) = match registry.stage_managed(&bytes, !preview).await {
                    Ok(value) => value,
                    Err(message) => {
                        return error(StatusCode::BAD_REQUEST, "invalid_plugin_package", &message)
                    }
                };
                if preview {
                    return output(StatusCode::OK, metadata);
                }
                let manifest = match serde_json::from_value(metadata["manifest"].clone()) {
                    Ok(manifest) => manifest,
                    Err(cause) => {
                        return error(
                            StatusCode::BAD_REQUEST,
                            "invalid_manifest",
                            &cause.to_string(),
                        )
                    }
                };
                if let Err(message) = registry.validate_instances(&manifest, None, providers) {
                    return error(StatusCode::BAD_REQUEST, "incompatible_channel", &message);
                }
                registry.install_managed(staging, metadata, expected).await
            }
        };
        return match result {
            Ok(plugin) => output(StatusCode::OK, json!({"plugin":plugin})),
            Err(message) => error(
                if message == "revision_conflict" {
                    StatusCode::CONFLICT
                } else {
                    StatusCode::BAD_REQUEST
                },
                if message == "revision_conflict" {
                    "revision_conflict"
                } else {
                    "validation_error"
                },
                &message,
            ),
        };
    }
    if method == Method::GET && path == "/admin/api/plugins" {
        let mut value = registry.list();
        let balances: Vec<_> = providers
            .iter()
            .map(|provider| {
                provider
                    .preferences
                    .get("balance_plugin")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or_else(|| registry.balance_plugin_id(provider).ok().flatten())
            })
            .collect();
        for plugin in value["plugins"].as_array_mut().unwrap() {
            let channels: Vec<_> = providers
                .iter()
                .zip(&balances)
                .filter(|(provider, balance)| {
                    if plugin["kind"] == "builtin" {
                        plugin["manifest"]["engine"].as_str() == Some(provider.engine.as_ref())
                    } else {
                        super::runtime::channel_plugin_id(provider) == plugin["id"].as_str()
                            || balance.as_deref() == plugin["id"].as_str()
                    }
                })
                .map(|(provider, _)| provider.name.as_ref())
                .collect();
            plugin["channels"] = json!(channels);
        }
        return output(StatusCode::OK, value);
    }
    if method == Method::GET && path == "/admin/api/interceptors" {
        return output(StatusCode::OK, registry.interceptors());
    }
    if method != Method::POST && method != Method::PUT {
        return error(
            StatusCode::METHOD_NOT_ALLOWED,
            "method_not_allowed",
            "Unsupported plugin API method",
        );
    }
    let bytes = match tokio::time::timeout(
        std::time::Duration::from_secs(10),
        to_bytes(request.into_body(), 1024 * 1024),
    )
    .await
    {
        Ok(Ok(bytes)) => bytes,
        _ => {
            return error(
                StatusCode::BAD_REQUEST,
                "invalid_body",
                "Request body is too large or unreadable",
            )
        }
    };
    let value: Value = if bytes.is_empty() {
        json!({})
    } else {
        match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(_) => {
                return error(
                    StatusCode::BAD_REQUEST,
                    "invalid_json",
                    "Expected a JSON request body",
                )
            }
        }
    };
    let result = if path == "/admin/api/plugins" && method == Method::POST {
        match serde_json::from_value::<Registration>(value) {
            Ok(registration) => match registry.validate_instances(
                &registration.manifest,
                Some(&registration.config),
                providers,
            ) {
                Ok(()) => registry
                    .register_checked(registration)
                    .await
                    .map(|plugin| json!({"plugin":plugin})),
                Err(message) => Err(message),
            },
            Err(_) => Err("invalid plugin registration fields".into()),
        }
    } else if path == "/admin/api/interceptors" && method == Method::PUT {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Update {
            expected_revision: u64,
            interceptors: Vec<HeaderInterceptor>,
        }
        match serde_json::from_value::<Update>(value) {
            Ok(update) => {
                registry.update_interceptors(update.expected_revision, update.interceptors)
            }
            Err(_) => Err("invalid interceptor update fields".into()),
        }
    } else if let Some(tail) = path.strip_prefix("/admin/api/plugins/") {
        let pieces: Vec<_> = tail.split('/').collect();
        if pieces.len() != 2 || method != Method::POST {
            return error(
                StatusCode::NOT_FOUND,
                "not_found",
                "Unknown plugin operation",
            );
        }
        let id = pieces[0];
        match pieces[1] {
            "enable" | "disable" => {
                if let Some(expected) = value.get("expected_revision").and_then(Value::as_u64) {
                    registry
                        .change_checked(id, expected, Some(pieces[1] == "enable"), None)
                        .await
                        .map(|plugin| json!({"plugin":plugin}))
                } else {
                    Err("expected_revision is required".into())
                }
            }
            "rollback" => {
                if let (Some(expected), Some(revision)) = (
                    value.get("expected_revision").and_then(Value::as_u64),
                    value.get("revision").and_then(Value::as_u64),
                ) {
                    match registry.validate_rollback_instances(id, revision, providers) {
                        Ok(()) => {}
                        Err(message) => {
                            return error(StatusCode::BAD_REQUEST, "incompatible_channel", &message)
                        }
                    }
                    registry
                        .change_checked(id, expected, None, Some(revision))
                        .await
                        .map(|plugin| json!({"plugin":plugin}))
                } else {
                    Err("expected_revision and revision are required".into())
                }
            }
            "oauth" => {
                let channel = value
                    .get("channel")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let mut config = providers
                    .iter()
                    .find(|provider| {
                        provider.name.as_ref() == channel
                            && super::runtime::channel_plugin_id(provider) == Some(id)
                    })
                    .and_then(|provider| provider.preferences.get("plugin_config"))
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                if let Some(overrides) = value.get("config").and_then(Value::as_object) {
                    for (key, value) in overrides {
                        if value != "***" {
                            config.insert(key.clone(), value.clone());
                        }
                    }
                }
                registry
                    .oauth(
                        id,
                        channel,
                        config,
                        value.get("oauth").cloned().unwrap_or(Value::Null),
                    )
                    .await
            }
            "models" => {
                let config = match value.get("config") {
                    None => Default::default(),
                    Some(Value::Object(config)) => config.clone(),
                    _ => {
                        return error(
                            StatusCode::BAD_REQUEST,
                            "validation_error",
                            "config must be an object",
                        )
                    }
                };
                let channel = value
                    .get("channel")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let provider = providers
                    .iter()
                    .find(|provider| provider.name.as_ref() == channel);
                let key = value
                    .get("account_ref")
                    .and_then(Value::as_str)
                    .or_else(|| {
                        provider.and_then(|provider| provider.api_keys.first().map(String::as_str))
                    })
                    .unwrap_or_default();
                registry
                    .models_authenticated(
                        oauth,
                        id,
                        channel,
                        config,
                        key,
                        provider
                            .and_then(|provider| provider.preferences.get("proxy"))
                            .and_then(Value::as_str),
                    )
                    .await
            }
            _ => {
                return error(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    "Unknown plugin operation",
                )
            }
        }
    } else {
        return error(
            StatusCode::NOT_FOUND,
            "not_found",
            "Unknown plugin API route",
        );
    };
    match result {
        Ok(value) => output(StatusCode::OK, value),
        Err(message) => {
            let (status, code) = match message.as_str() {
                "revision_conflict" => (StatusCode::CONFLICT, "revision_conflict"),
                "plugin_not_found" | "revision_not_found" => (StatusCode::NOT_FOUND, "not_found"),
                _ => (StatusCode::BAD_REQUEST, "validation_error"),
            };
            error(status, code, &message)
        }
    }
}
fn error(status: StatusCode, code: &str, message: &str) -> Response<Body> {
    output(status, json!({"error":{"code":code,"message":message}}))
}
fn output(status: StatusCode, value: Value) -> Response<Body> {
    let mut response = Response::new(Body::from(value.to_string()));
    *response.status_mut() = status;
    response.headers_mut().insert(
        "content-type",
        "application/json; charset=utf-8".parse().unwrap(),
    );
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    response
}
