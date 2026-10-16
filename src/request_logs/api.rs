//! These handlers are delegated only after admin session and mutation-CSRF checks.
use super::store::{CleanupRequest, Failure, Filters, Operation, Reply};
use super::{CaptureField, RequestLogs};
use axum::body::{to_bytes, Body};
use axum::extract::Request;
use axum::http::{HeaderValue, Method, Response, StatusCode, Uri};
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::{json, Value};

pub(crate) fn supports(path: &str) -> bool {
    let path = path.trim_end_matches('/');
    path == "/admin/api/log-settings"
        || path == "/admin/api/request-logs"
        || path.starts_with("/admin/api/request-logs/")
}
pub(crate) async fn handle(logs: &RequestLogs, request: Request) -> Response<Body> {
    match handle_inner(logs, request).await {
        Ok(response) => response,
        Err(error) => json_response(
            error.status,
            json!({"error":{"code":error.code,"message":error.message}}),
        ),
    }
}
async fn handle_inner(logs: &RequestLogs, request: Request) -> Result<Response<Body>, Failure> {
    let path = request.uri().path().trim_end_matches('/').to_owned();
    let method = request.method().clone();
    if path == "/admin/api/log-settings" {
        if method == Method::GET {
            let snapshot = logs.settings();
            let mut settings = serde_json::to_value(&snapshot.settings).unwrap();
            if !snapshot.runtime_rotation_managed {
                settings.as_object_mut().unwrap().remove("runtime");
            }
            return Ok(json_response(
                StatusCode::OK,
                json!({"revision":snapshot.revision,"settings":settings,"backend_enabled":snapshot.backend_enabled,"runtime_rotation_managed":snapshot.runtime_rotation_managed,"capture_fields":CaptureField::ALL,"status":logs.status()}),
            ));
        }
        if method == Method::PUT {
            #[derive(Deserialize)]
            struct Update {
                revision: u64,
                settings: Value,
            }
            let update: Update = read_json(request).await?;
            return reply_json(
                logs.query(Operation::UpdateSettings {
                    revision: update.revision,
                    settings: update.settings,
                })
                .await?,
            );
        }
        return Err(method_error());
    }
    if path == "/admin/api/request-logs/cleanup" {
        if method != Method::POST {
            return Err(method_error());
        }
        let input: CleanupRequest = read_json(request).await?;
        return reply_json(logs.query(Operation::Cleanup(input)).await?);
    }
    if path == "/admin/api/request-logs" {
        if method != Method::GET {
            return Err(method_error());
        }
        let filters = filters(request.uri())?;
        let page = number::<u64>(request.uri(), "page")?.unwrap_or(1).max(1);
        let page_size = number::<u64>(request.uri(), "page_size")?
            .unwrap_or(50)
            .clamp(1, 200);
        return reply_json(
            logs.query(Operation::List {
                filters,
                page,
                page_size,
            })
            .await?,
        );
    }
    let suffix = path
        .strip_prefix("/admin/api/request-logs/")
        .ok_or_else(not_found)?;
    let parts = suffix.split('/').collect::<Vec<_>>();
    if method != Method::GET {
        return Err(method_error());
    }
    if parts.len() == 1 && !parts[0].is_empty() {
        return reply_json(logs.query(Operation::Detail(parts[0].into())).await?);
    }
    if parts.len() != 2 || !matches!(parts[1], "content" | "download") {
        return Err(not_found());
    }
    let field = query(request.uri(), "field")
        .and_then(|value| CaptureField::parse(&value))
        .ok_or_else(|| Failure::bad("field must be one of the eight capture field names"))?;
    let attempt = number::<u32>(request.uri(), "attempt")?;
    let download = parts[1] == "download";
    let limit = if download {
        None
    } else {
        Some(
            number::<usize>(request.uri(), "limit")?
                .unwrap_or(65_536)
                .clamp(1, 65_536),
        )
    };
    let Reply::Content {
        mut bytes,
        mut info,
        mut truncated,
    } = logs
        .query(Operation::Content {
            id: parts[0].into(),
            field,
            attempt,
            limit,
        })
        .await?
    else {
        return Err(Failure::unavailable());
    };
    if download {
        let complete = info["complete"].as_bool().unwrap_or(false);
        let suffix = if complete { "" } else { "-partial" };
        let name = format!(
            "request-log-{}{}{}.bin",
            field.as_str(),
            attempt
                .map(|index| format!("-attempt-{index}"))
                .unwrap_or_default(),
            suffix
        );
        let mut response = Response::new(Body::from(bytes));
        response.headers_mut().insert(
            "content-type",
            HeaderValue::from_static("application/octet-stream"),
        );
        response.headers_mut().insert(
            "content-disposition",
            HeaderValue::from_str(&format!("attachment; filename=\"{name}\"")).unwrap(),
        );
        response.headers_mut().insert(
            "x-content-type-options",
            HeaderValue::from_static("nosniff"),
        );
        response.headers_mut().insert(
            "x-log-content-complete",
            HeaderValue::from_static(if complete { "true" } else { "false" }),
        );
        response
            .headers_mut()
            .insert("cache-control", HeaderValue::from_static("no-store"));
        return Ok(response);
    }
    if let Err(error) = std::str::from_utf8(&bytes) {
        bytes.truncate(error.valid_up_to());
        truncated = true;
    }
    info["content"] = json!(String::from_utf8_lossy(&bytes));
    info["encoding"] = json!("utf-8");
    info["preview_truncated"] = json!(truncated);
    Ok(json_response(StatusCode::OK, info))
}
fn query(uri: &Uri, name: &str) -> Option<String> {
    url::form_urlencoded::parse(uri.query()?.as_bytes())
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
        .filter(|value| !value.is_empty())
}
fn number<T: std::str::FromStr>(uri: &Uri, name: &str) -> Result<Option<T>, Failure> {
    query(uri, name)
        .map(|value| {
            value
                .parse::<T>()
                .map_err(|_| Failure::bad(format!("Invalid numeric query parameter: {name}")))
        })
        .transpose()
}
fn filters(uri: &Uri) -> Result<Filters, Failure> {
    let filters = Filters {
        request_id: query(uri, "request_id"),
        channel: query(uri, "channel"),
        model: query(uri, "model"),
        status: number(uri, "status")?,
        from_ms: number(uri, "from_ms")?,
        to_ms: number(uri, "to_ms")?,
        ids: None,
    };
    if let (Some(from), Some(to)) = (filters.from_ms, filters.to_ms) {
        if from > to {
            return Err(Failure::bad("from_ms must not exceed to_ms"));
        }
    }
    Ok(filters)
}
async fn read_json<T: serde::de::DeserializeOwned>(request: Request) -> Result<T, Failure> {
    let bytes = to_bytes(request.into_body(), 262_144)
        .await
        .map_err(|_| Failure::bad("Log management body is too large"))?;
    serde_json::from_slice(&bytes).map_err(|_| Failure::bad("Invalid log management JSON body"))
}
fn reply_json(reply: Reply) -> Result<Response<Body>, Failure> {
    match reply {
        Reply::Json(value) => Ok(json_response(StatusCode::OK, value)),
        _ => Err(Failure::unavailable()),
    }
}
fn json_response(status: StatusCode, value: Value) -> Response<Body> {
    (
        status,
        [
            ("content-type", "application/json; charset=utf-8"),
            ("cache-control", "no-store"),
            ("x-content-type-options", "nosniff"),
        ],
        value.to_string(),
    )
        .into_response()
}
fn method_error() -> Failure {
    Failure::new(
        StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
        "Method not supported for this request-log resource",
    )
}
fn not_found() -> Failure {
    Failure::new(
        StatusCode::NOT_FOUND,
        "request_log_route_not_found",
        "Unknown request-log endpoint",
    )
}
