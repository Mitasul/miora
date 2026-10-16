use super::error;
use axum::body::Body;
use axum::http::{Response, StatusCode, Uri};
use serde_json::Value;

pub(super) fn read(uri: &Uri) -> Result<Value, Response<Body>> {
    let params: std::collections::HashMap<_, _> =
        url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes())
            .into_owned()
            .collect();
    let number = |name: &str, default: u64| -> Result<u64, Response<Body>> {
        params
            .get(name)
            .map(|value| {
                value.parse().map_err(|_| {
                    error(
                        StatusCode::BAD_REQUEST,
                        "invalid_log_query",
                        "Log cursor and limit must be nonnegative integers",
                    )
                })
            })
            .unwrap_or(Ok(default))
    };
    let level = params.get("level").map(String::as_str).unwrap_or("");
    if !matches!(level, "" | "INFO" | "WARN" | "ERROR" | "DEBUG" | "CRITICAL") {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "invalid_log_level",
            "Unknown backend log level",
        ));
    }
    Ok(crate::observability::backend_logs::read(
        number("after", 0)?,
        level,
        params.get("q").map(String::as_str).unwrap_or(""),
        number("limit", 200)?.clamp(1, 500) as usize,
    ))
}
