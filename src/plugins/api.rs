//! Internal adapter. The caller MUST authenticate an administrator session and
//! verify CSRF before calling this handler; it never accepts a provider API key.
use super::interceptors::HeaderInterceptor;
use super::registry::{PluginRegistry, Registration};
use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, Response, StatusCode};
use serde::Deserialize;
use serde_json::{json, Value};

pub(crate) async fn handle(registry: &PluginRegistry, request: Request<Body>) -> Response<Body> {
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
    if path == "/admin/api/plugins/upload" && method == Method::POST {
        let registration = match super::upload::read(request).await {
            Ok(registration) => registration,
            Err((status, message)) => return error(status, "invalid_plugin_file", &message),
        };
        return match registry.register_checked(registration).await {
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
        return output(StatusCode::OK, registry.list());
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
            Ok(registration) => registry
                .register_checked(registration)
                .await
                .map(|plugin| json!({"plugin":plugin})),
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
                    registry
                        .change_checked(id, expected, None, Some(revision))
                        .await
                        .map(|plugin| json!({"plugin":plugin}))
                } else {
                    Err("expected_revision and revision are required".into())
                }
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
                registry.models(id, config).await
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
