use super::{bounded_body, error, management};
use crate::config::snapshot::{runtime_provider, RawProvider};
use crate::config::source::RuntimeConfigSource;
use crate::runtime::context::AppState;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{Response, StatusCode, Uri};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::{atomic::AtomicUsize, Arc};

type ApiResult = Result<Value, Response<Body>>;

async fn configuration(state: &AppState) -> Result<(String, Value), Response<Body>> {
    if matches!(state.config_publisher.source, RuntimeConfigSource::File(_)) {
        management::source_config(state).await
    } else {
        let snapshot = state
            .runtime
            .snapshot()
            .await
            .ok_or_else(management::unavailable)?;
        Ok((
            snapshot.revision.to_string(),
            snapshot.api_config.as_ref().clone(),
        ))
    }
}

fn channel_names(config: &Value) -> Vec<String> {
    config["providers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row["provider"].as_str().map(str::to_owned))
        .collect()
}

fn storage_error(message: String) -> Response<Body> {
    error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "balance_storage_failed",
        &message,
    )
}

pub(super) async fn read(state: &AppState, uri: &Uri) -> ApiResult {
    let (revision, config) = configuration(state).await?;
    let names = channel_names(&config);
    let provider = url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes())
        .find(|(key, _)| key == "provider")
        .map(|(_, value)| value.into_owned());
    if provider.as_ref().is_some_and(|name| !names.contains(name)) {
        return Err(error(
            StatusCode::NOT_FOUND,
            "channel_not_found",
            "Channel not found",
        ));
    }
    let mut values = state.channel_balances.read().await.map_err(storage_error)?;
    values.retain(|name, _| {
        names.contains(name) && provider.as_ref().is_none_or(|wanted| wanted == name)
    });
    for value in values.values_mut() {
        value["configuration_changed"] = json!(value["config_revision"] != revision);
    }
    Ok(json!({"balances":values,"revision":revision}))
}

#[derive(Deserialize)]
struct BalanceQuery {
    provider: String,
}

pub(super) async fn refresh(state: &AppState, request: Request) -> ApiResult {
    let raw = bounded_body(request.into_body(), 262_144).await?;
    let input: BalanceQuery = serde_json::from_slice(&raw).map_err(|_| {
        error(
            StatusCode::BAD_REQUEST,
            "invalid_balance_query",
            "Expected a provider name",
        )
    })?;
    let (revision, config) = configuration(state).await?;
    let source = config["providers"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|row| row["provider"].as_str() == Some(&input.provider))
        .ok_or_else(|| {
            error(
                StatusCode::NOT_FOUND,
                "channel_not_found",
                "Channel not found",
            )
        })?;
    let compiled =
        crate::config::compiler::compile_provider(source).ok_or_else(management::unavailable)?;
    let raw: RawProvider =
        serde_json::from_value(compiled).map_err(|_| management::unavailable())?;
    let provider = runtime_provider(raw, Arc::new(AtomicUsize::new(0)));
    let proxy = provider
        .preferences
        .get("proxy")
        .or_else(|| config.pointer("/preferences/proxy"))
        .and_then(Value::as_str);
    let result = crate::providers::balances::query_channel(state, &provider, proxy).await;
    state
        .channel_balances
        .save(&input.provider, &revision, result, &channel_names(&config))
        .await
        .map_err(storage_error)
}
