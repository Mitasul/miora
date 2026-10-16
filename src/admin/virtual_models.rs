use super::{bounded_body, error, management};
use crate::runtime::context::AppState;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{Response, StatusCode};
use serde::Deserialize;
use serde_json::{json, Value};

type ApiResult = Result<Value, Response<Body>>;

pub(super) async fn read(state: &AppState) -> ApiResult {
    let (revision, config) = management::source_config(state).await?;
    let providers = config["providers"].as_array().into_iter().flatten().filter_map(|source| {
        let provider = crate::config::compiler::compile_provider(source)?;
        Some(json!({"provider":provider["name"],"enabled":source.get("enabled").and_then(Value::as_bool).unwrap_or(true),"models":provider["models"].as_object()?.keys().collect::<Vec<_>>()}))
    }).collect::<Vec<_>>();
    Ok(
        json!({"revision":revision,"virtual_models":config.get("virtual_models").cloned().unwrap_or_else(|| json!({})),"providers":providers}),
    )
}

#[derive(Deserialize)]
struct Update {
    revision: String,
    virtual_models: Value,
}

pub(super) async fn write(state: &AppState, request: Request) -> ApiResult {
    let raw = bounded_body(request.into_body(), 262_144).await?;
    let input: Update = serde_json::from_slice(&raw).map_err(|_| {
        error(
            StatusCode::BAD_REQUEST,
            "invalid_virtual_models",
            "Expected revision and virtual_models",
        )
    })?;
    let _guard = state.admin.config_lock().try_lock().map_err(|_| {
        error(
            StatusCode::CONFLICT,
            "configuration_busy",
            "Another configuration write is in progress",
        )
    })?;
    state
        .config_publisher
        .apply_patch_revision(
            &json!({"virtual_models":input.virtual_models}),
            &input.revision,
        )
        .await
        .map_err(management::configuration_error)?;
    state
        .runtime
        .refresh()
        .await
        .map_err(|_| management::unavailable())?;
    Ok(json!({"ok":true,"persistence":"source_file"}))
}
