//! Admin endpoints never accept or upgrade gateway Bearer credentials.
//! Call before the legacy CORS/proxy pipeline. See docs/admin.md.
mod accounts;
mod auth;
mod backend_logs;
mod balances;
mod channel_preview;
mod management;
mod testing;
mod virtual_models;
mod workspace;
pub(crate) use auth::AdminState;

use crate::runtime::context::AppState;
use axum::body::{to_bytes, Body};
use axum::extract::Request;
use axum::http::{HeaderMap, HeaderValue, Method, Response, StatusCode};
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

pub(crate) fn supports(path: &str) -> bool {
    path == "/admin/api" || path.starts_with("/admin/api/")
}

pub(crate) async fn handle(state: &AppState, request: Request) -> Response<Body> {
    let mut response = handle_inner(state, request).await;
    let headers = response.headers_mut();
    headers.insert("cache-control", HeaderValue::from_static("no-store"));
    headers.insert("pragma", HeaderValue::from_static("no-cache"));
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert(
        "content-security-policy",
        HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'"),
    );
    // Even a delegated handler must not enable browser cross-origin admin access.
    for name in [
        "access-control-allow-origin",
        "access-control-allow-credentials",
        "access-control-allow-headers",
        "access-control-allow-methods",
    ] {
        headers.remove(name);
    }
    response
}

async fn handle_inner(state: &AppState, request: Request) -> Response<Body> {
    let path = request.uri().path().trim_end_matches('/').to_owned();
    let method = request.method().clone();
    if path == "/admin/api/login" {
        if method != Method::POST {
            return error(
                StatusCode::METHOD_NOT_ALLOWED,
                "method_not_allowed",
                "Use POST to log in",
            );
        }
        return login(state, request).await;
    }
    // A bearer, a singleton API key, or any role string is irrelevant here.
    let Some(session) = state.admin.session(request.headers()) else {
        return error(
            StatusCode::UNAUTHORIZED,
            "authentication_required",
            "Administrator login required",
        );
    };
    if method != Method::GET
        && method != Method::HEAD
        && (!state.admin.check_origin(request.headers())
            || !auth::csrf_valid(request.headers(), &session))
    {
        return error(
            StatusCode::FORBIDDEN,
            "csrf_rejected",
            "Same-origin request and valid X-CSRF-Token required",
        );
    }
    if path == "/admin/api/session" {
        return if method == Method::GET {
            response(StatusCode::OK, state.admin.session_value(&session))
        } else {
            error(
                StatusCode::METHOD_NOT_ALLOWED,
                "method_not_allowed",
                "Use GET to read the session",
            )
        };
    }
    if path == "/admin/api/logout" {
        if method != Method::POST {
            return error(
                StatusCode::METHOD_NOT_ALLOWED,
                "method_not_allowed",
                "Use POST to log out",
            );
        }
        state.admin.logout(request.headers());
        state.admin.audit("logout", "success");
        let mut out = response(StatusCode::OK, json!({"ok":true}));
        out.headers_mut().insert(
            "set-cookie",
            HeaderValue::from_str(&state.admin.cookie("")).unwrap(),
        );
        out.headers_mut().append(
            "set-cookie",
            HeaderValue::from_str(&state.admin.clear_legacy_cookie()).unwrap(),
        );
        return out;
    }
    // Bound every mutation (including delegated plugin endpoints) before parsing.
    let request = if method != Method::GET && method != Method::HEAD {
        let limit = match mutation_body_limit(&path, &method, request.headers()) {
            Ok(limit) => limit,
            Err(out) => return out,
        };
        let (parts, body) = request.into_parts();
        let bytes = match bounded_body(body, limit).await {
            Ok(bytes) => bytes,
            Err(out) => return out,
        };
        Request::from_parts(parts, Body::from(bytes))
    } else {
        request
    };
    let mutation = method != Method::GET && method != Method::HEAD;
    let out = if crate::request_logs::api::supports(&path) {
        crate::request_logs::api::handle(&state.request_logs, request).await
    } else if path == "/admin/api/test/stream" && method == Method::POST {
        testing::stream(state, request).await
    } else if path == "/admin/api/plugins"
        || path.starts_with("/admin/api/plugins/")
        || path == "/admin/api/interceptors"
        || path.starts_with("/admin/api/interceptors/")
    {
        let snapshot = state.runtime.snapshot().await;
        let providers = snapshot
            .as_ref()
            .map(|snapshot| snapshot.providers.as_slice())
            .unwrap_or_default();
        crate::plugins::api::handle(
            state.plugins.as_ref(),
            request,
            providers,
            &state.codex_oauth,
        )
        .await
    } else {
        management::handle(state, request, &path).await
    };
    if mutation {
        // Only fixed route families, never untrusted path suffixes or plugin identifiers.
        let action = if path == "/admin/api/workspace" {
            "workspace_save"
        } else if path == "/admin/api/test" || path == "/admin/api/test/stream" {
            "model_test"
        } else if path == "/admin/api/log-settings" {
            "request_log_settings_update"
        } else if path.starts_with("/admin/api/request-logs") {
            "request_log_cleanup"
        } else if path.starts_with("/admin/api/plugins") {
            "plugins_mutation"
        } else if path.starts_with("/admin/api/interceptors") {
            "interceptors_mutation"
        } else if path.starts_with("/admin/api/settings") {
            "settings_mutation"
        } else if path.starts_with("/admin/api/api-keys") {
            "api_keys_mutation"
        } else if path == "/admin/api/channels/preview" {
            "channel_preview"
        } else if path == "/admin/api/channels/parameter-schema" {
            "channel_parameter_schema"
        } else if path.starts_with("/admin/api/channels") {
            "channels_mutation"
        } else if path == "/admin/api/update" {
            "system_update"
        } else {
            "unknown_mutation"
        };
        state.admin.audit(
            action,
            if out.status().is_success() {
                "success"
            } else {
                "rejected"
            },
        );
    }
    out
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Credentials {
    username: String,
    password: String,
}

async fn login(state: &AppState, request: Request) -> Response<Body> {
    if !state.admin.configured() {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "admin_not_configured",
            "Administrator credentials are not configured",
        );
    }
    if !state.admin.check_origin(request.headers()) {
        return error(
            StatusCode::FORBIDDEN,
            "origin_rejected",
            "Same-origin request required",
        );
    }
    if !state.admin.login_allowed(&state.admin.client_id(&request)) {
        return rate_limited();
    }
    if !json_content_type(request.headers()) {
        return error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "json_required",
            "Content-Type application/json required",
        );
    }
    let headers = request.headers().clone();
    let raw = match bounded_body(request.into_body(), 16_384).await {
        Ok(raw) => raw,
        Err(out) => return out,
    };
    let input = match serde_json::from_slice::<Credentials>(&raw) {
        Ok(input)
            if !input.username.is_empty()
                && input.username.len() <= 128
                && input.password.len() <= 1024 =>
        {
            input
        }
        _ => {
            return error(
                StatusCode::BAD_REQUEST,
                "invalid_login",
                "Invalid login request",
            )
        }
    };
    match state.admin.verify(input.username, input.password).await {
        Ok(true) => {}
        Ok(false) => {
            state.admin.audit("login", "invalid_credentials");
            return error(
                StatusCode::UNAUTHORIZED,
                "invalid_credentials",
                "Invalid username or password",
            );
        }
        Err(StatusCode::TOO_MANY_REQUESTS) => return rate_limited(),
        Err(_) => {
            return error(
                StatusCode::SERVICE_UNAVAILABLE,
                "authentication_unavailable",
                "Authentication temporarily unavailable",
            )
        }
    }
    let (token, session) = match state.admin.create_session(&headers) {
        Ok(result) => result,
        Err(_) => {
            return error(
                StatusCode::SERVICE_UNAVAILABLE,
                "session_capacity",
                "Session service temporarily unavailable",
            )
        }
    };
    state.admin.audit("login", "success");
    let mut out = response(StatusCode::OK, state.admin.session_value(&session));
    out.headers_mut().insert(
        "set-cookie",
        HeaderValue::from_str(&state.admin.cookie(&token)).unwrap(),
    );
    out.headers_mut().append(
        "set-cookie",
        HeaderValue::from_str(&state.admin.clear_legacy_cookie()).unwrap(),
    );
    out
}

fn rate_limited() -> Response<Body> {
    let mut out = error(
        StatusCode::TOO_MANY_REQUESTS,
        "login_rate_limited",
        "Too many login attempts; retry later",
    );
    out.headers_mut()
        .insert("retry-after", HeaderValue::from_static("60"));
    out
}

fn json_content_type(headers: &HeaderMap) -> bool {
    content_type_is(headers, "application/json")
}

fn content_type_is(headers: &HeaderMap, expected: &str) -> bool {
    headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case(expected))
}

fn mutation_body_limit(
    path: &str,
    method: &Method,
    headers: &HeaderMap,
) -> Result<usize, Response<Body>> {
    if matches!(
        path,
        "/admin/api/plugins/upload" | "/admin/api/plugins/preview"
    ) && method == Method::POST
    {
        return if content_type_is(headers, "multipart/form-data") {
            Ok(17 * 1024 * 1024)
        } else {
            Err(error(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "multipart_required",
                "Content-Type multipart/form-data required",
            ))
        };
    }
    if json_content_type(headers) {
        Ok(if path == "/admin/api/workspace" && method == Method::PUT {
            6 * 1024 * 1024 + 4096
        } else {
            262_144
        })
    } else {
        Err(error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "json_required",
            "Content-Type application/json required",
        ))
    }
}

pub(super) async fn bounded_body(body: Body, limit: usize) -> Result<bytes::Bytes, Response<Body>> {
    match tokio::time::timeout(Duration::from_secs(10), to_bytes(body, limit)).await {
        Ok(Ok(bytes)) => Ok(bytes),
        Ok(Err(_)) => Err(error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "body_too_large",
            "Request body exceeds the allowed limit",
        )),
        Err(_) => Err(error(
            StatusCode::REQUEST_TIMEOUT,
            "body_timeout",
            "Request body upload timed out",
        )),
    }
}

pub(super) fn response(status: StatusCode, value: Value) -> Response<Body> {
    (
        status,
        [("content-type", "application/json; charset=utf-8")],
        value.to_string(),
    )
        .into_response()
}
pub(super) fn error(status: StatusCode, code: &str, message: &str) -> Response<Body> {
    response(status, json!({"error":{"code":code,"message":message}}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multipart_is_limited_to_the_exact_post_upload_and_preview_routes() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "content-type",
            HeaderValue::from_static("multipart/form-data; boundary=test"),
        );
        assert_eq!(
            mutation_body_limit("/admin/api/plugins/upload", &Method::POST, &headers).unwrap(),
            17 * 1024 * 1024
        );
        assert_eq!(
            mutation_body_limit("/admin/api/plugins/preview", &Method::POST, &headers).unwrap(),
            17 * 1024 * 1024
        );
        for path in [
            "/admin/api/plugins",
            "/admin/api/channels",
            "/admin/api/plugins/upload-evil",
        ] {
            assert_eq!(
                mutation_body_limit(path, &Method::POST, &headers)
                    .unwrap_err()
                    .status(),
                StatusCode::UNSUPPORTED_MEDIA_TYPE
            );
        }
        assert!(mutation_body_limit("/admin/api/plugins/upload", &Method::PUT, &headers).is_err());
        headers.insert(
            "content-type",
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
        assert_eq!(
            mutation_body_limit("/admin/api/channels", &Method::POST, &headers).unwrap(),
            262_144
        );
        assert!(mutation_body_limit("/admin/api/plugins/upload", &Method::POST, &headers).is_err());
    }
    #[test]
    fn dispatch_does_not_claim_similarly_named_routes() {
        assert!(supports("/admin/api/session"));
        assert!(supports("/admin/api"));
        assert!(!supports("/admin/api-evil"));
        assert!(!supports("/v1/api-keys"));
    }
    #[tokio::test]
    async fn body_limits_are_enforced() {
        assert_eq!(
            bounded_body(Body::from("large"), 2)
                .await
                .unwrap_err()
                .status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
    }
}
