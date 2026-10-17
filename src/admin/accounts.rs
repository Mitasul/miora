use super::{bounded_body, error, management};
use crate::runtime::context::AppState;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{Method, Response, StatusCode, Uri};
use serde::Deserialize;
use serde_json::{json, Value};

type ApiResult = Result<Value, Response<Body>>;

fn channel<'a>(config: &'a Value, name: &str) -> Result<&'a Value, Response<Body>> {
    config["providers"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|row| row["provider"].as_str() == Some(name))
        .ok_or_else(|| {
            error(
                StatusCode::NOT_FOUND,
                "channel_not_found",
                "Channel not found",
            )
        })
}

fn mask_credential(key: &str) -> String {
    let trimmed = key.trim();
    if let Some((account, _)) = trimmed.split_once(',') {
        let account = account.trim();
        if !account.is_empty() {
            return account.to_owned();
        }
    }
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= 10 {
        "••••••••".into()
    } else {
        let prefix: String = chars[..6].iter().collect();
        let suffix: String = chars[chars.len() - 4..].iter().collect();
        format!("{prefix}••••{suffix}")
    }
}

pub(super) async fn list(state: &AppState, uri: &Uri) -> ApiResult {
    let params: std::collections::HashMap<String, String> =
        url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes())
            .into_owned()
            .collect();
    if params.get("oauth").map(String::as_str) == Some("authorize") {
        let provider = params.get("provider").map(String::as_str);
        let redirect_uri = params.get("redirect_uri").map(String::as_str);
        return Ok(state
            .codex_oauth
            .start_oauth_login(provider, redirect_uri)
            .await);
    }
    if params.get("oauth").map(String::as_str) == Some("status") {
        let st = params.get("state").map(String::as_str).unwrap_or_default();
        return Ok(state.codex_oauth.oauth_session_status(st).await);
    }
    let name = params.get("provider").cloned().unwrap_or_default();
    let (revision, config) = management::source_config(state).await?;
    let source = channel(&config, &name)?;
    let mut expanded = source.clone();
    crate::config::compiler::expand_environment(&mut expanded);
    let keys = crate::config::snapshot::provider_api_keys(&expanded["api"]);
    let engine = source["engine"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| {
            crate::config::compiler::infer_engine(source["base_url"].as_str().unwrap_or_default())
        });
    let now = tokio::time::Instant::now();
    let mut rows = Vec::new();
    for (index, key) in keys.iter().enumerate() {
        let position = index + 1;
        let disabled = source["disabled_key_positions"]
            .as_array()
            .is_some_and(|positions| positions.contains(&json!(position)));
        let cooldown = state
            .runtime
            .scheduling
            .key_cooldowns
            .lock()
            .await
            .get(&(name.clone(), key.clone()))
            .map(|until| until.saturating_duration_since(now).as_secs())
            .unwrap_or(0);
        let failures = state
            .runtime
            .scheduling
            .key_failures
            .lock()
            .await
            .get(&(name.clone(), key.clone()))
            .copied()
            .unwrap_or(0);
        let oauth = if matches!(engine.as_str(), "codex" | "claude" | "antigravity")
            || key.starts_with("eyJ")
        {
            state.codex_oauth.status(key).await
        } else {
            Value::Null
        };
        rows.push(json!({
            "position": position,
            "enabled": !disabled,
            "credential": mask_credential(key),
            "cooldown_seconds": cooldown,
            "failure_count": failures,
            "oauth": oauth,
        }));
    }
    Ok(
        json!({"provider":name,"revision":revision,"engine":engine,"keys":rows,"failure_scope":"current_process","writable":true}),
    )
}

#[derive(Deserialize)]
struct OAuthExchangeInput {
    code: String,
    #[serde(default)]
    code_verifier: String,
    #[serde(default)]
    provider: String,
    state: Option<String>,
    redirect_uri: Option<String>,
}

#[derive(Deserialize)]
struct Mutation {
    #[serde(default)]
    revision: String,
    #[serde(default)]
    provider: String,
    #[serde(default)]
    position: usize,
    enabled: Option<bool>,
    #[serde(default)]
    remove: bool,
    #[serde(default)]
    refresh: bool,
    #[serde(default)]
    oauth_only: bool,
    import: Option<Value>,
    oauth_exchange: Option<OAuthExchangeInput>,
}

pub(super) async fn change(state: &AppState, request: Request) -> ApiResult {
    let method = request.method().clone();
    let raw = bounded_body(request.into_body(), 262_144).await?;
    let input: Mutation = serde_json::from_slice(&raw).map_err(|_| {
        error(
            StatusCode::BAD_REQUEST,
            "invalid_account",
            "Expected provider, revision and account operation",
        )
    })?;
    if method == Method::POST && input.oauth_only {
        if let Some(imported) = &input.import {
            let credentials = import_accounts(state, imported, &input.provider).await?;
            return Ok(json!({"ok":true,"credentials":credentials}));
        }
        let exchange = input.oauth_exchange.as_ref().ok_or_else(|| {
            error(
                StatusCode::BAD_REQUEST,
                "invalid_oauth_exchange",
                "Expected oauth_exchange payload",
            )
        })?;
        let provider_hint = if !exchange.provider.is_empty() {
            exchange.provider.as_str()
        } else if !input.provider.is_empty() {
            input.provider.as_str()
        } else {
            "codex"
        };
        let raw_key = state
            .codex_oauth
            .exchange_code_with_provider(
                provider_hint,
                &exchange.code,
                &exchange.code_verifier,
                exchange.state.as_deref(),
                exchange.redirect_uri.as_deref(),
                None,
            )
            .await
            .map_err(|message| error(StatusCode::BAD_GATEWAY, "oauth_exchange_failed", &message))?;
        let oauth = state.codex_oauth.status(&raw_key).await;
        return Ok(json!({"ok":true,"credential":raw_key,"oauth":oauth}));
    }
    let _guard = state.admin.config_lock().try_lock().map_err(|_| {
        error(
            StatusCode::CONFLICT,
            "configuration_busy",
            "Another configuration write is in progress",
        )
    })?;
    let (revision, mut config) = management::source_config(state).await?;
    if input.revision != revision {
        return Err(error(
            StatusCode::CONFLICT,
            "revision_conflict",
            "Refresh accounts before retrying",
        ));
    }
    let source = channel(&config, &input.provider)?;
    let engine = source["engine"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| {
            crate::config::compiler::infer_engine(source["base_url"].as_str().unwrap_or_default())
        });
    let mut expanded = source.clone();
    crate::config::compiler::expand_environment(&mut expanded);
    let keys = crate::config::snapshot::provider_api_keys(&expanded["api"]);
    if input.refresh {
        let key = input
            .position
            .checked_sub(1)
            .and_then(|index| keys.get(index))
            .ok_or_else(|| {
                error(
                    StatusCode::NOT_FOUND,
                    "account_not_found",
                    "Key position not found",
                )
            })?;
        if !matches!(engine.as_str(), "codex" | "claude" | "antigravity") || !key.contains(',') {
            return Err(error(
                StatusCode::BAD_REQUEST,
                "oauth_unsupported",
                "Only OAuth refresh-token accounts can be refreshed here",
            ));
        }
        let proxy = expanded
            .pointer("/preferences/proxy")
            .and_then(Value::as_str);
        state
            .codex_oauth
            .refresh_now(key, proxy)
            .await
            .map_err(|_| {
                error(
                    StatusCode::BAD_GATEWAY,
                    "oauth_refresh_failed",
                    "OAuth refresh failed; check account validity and upstream connectivity",
                )
            })?;
        return Ok(json!({"ok":true,"oauth":state.codex_oauth.status(key).await}));
    }
    let row = config["providers"]
        .as_array_mut()
        .expect("channel array")
        .iter_mut()
        .find(|row| row["provider"] == input.provider)
        .expect("selected channel");
    if method == Method::POST {
        let imported = if let Some(exchange) = &input.oauth_exchange {
            let proxy = expanded
                .pointer("/preferences/proxy")
                .and_then(Value::as_str);
            let provider_hint = if !exchange.provider.is_empty() {
                exchange.provider.as_str()
            } else {
                engine.as_str()
            };
            let raw_key = state
                .codex_oauth
                .exchange_code_with_provider(
                    provider_hint,
                    &exchange.code,
                    &exchange.code_verifier,
                    exchange.state.as_deref(),
                    exchange.redirect_uri.as_deref(),
                    proxy,
                )
                .await
                .map_err(|message| {
                    error(StatusCode::BAD_GATEWAY, "oauth_exchange_failed", &message)
                })?;
            vec![raw_key]
        } else {
            let import_val = input.import.as_ref().ok_or_else(|| {
                error(
                    StatusCode::BAD_REQUEST,
                    "invalid_import",
                    "Provide key strings or an account JSON document",
                )
            })?;
            import_accounts(state, import_val, &engine).await?
        };
        let mut existing = crate::config::snapshot::provider_api_keys(&row["api"]);
        if existing.len() + imported.len() > 4096 {
            return Err(error(
                StatusCode::BAD_REQUEST,
                "too_many_keys",
                "At most 4096 keys per channel",
            ));
        }
        for key in imported {
            if !existing.contains(&key) {
                existing.push(key);
            }
        }
        row["api"] = json!(existing);
    } else if input.remove {
        if input.position == 0 || input.position > keys.len() {
            return Err(error(
                StatusCode::NOT_FOUND,
                "account_not_found",
                "Key position not found",
            ));
        }
        let mut existing = crate::config::snapshot::provider_api_keys(&row["api"]);
        let removed_key = existing.remove(input.position - 1);
        state.codex_oauth.clear(&removed_key).await;
        row["api"] = json!(existing);
        if let Some(disabled) = row["disabled_key_positions"].as_array() {
            let updated: Vec<Value> = disabled
                .iter()
                .filter_map(Value::as_u64)
                .filter(|&pos| pos != input.position as u64)
                .map(|pos| {
                    if pos > input.position as u64 {
                        json!(pos - 1)
                    } else {
                        json!(pos)
                    }
                })
                .collect();
            row["disabled_key_positions"] = json!(updated);
        }
    } else {
        if input.position == 0 || input.position > keys.len() {
            return Err(error(
                StatusCode::NOT_FOUND,
                "account_not_found",
                "Key position not found",
            ));
        }
        let enabled = input.enabled.ok_or_else(|| {
            error(
                StatusCode::BAD_REQUEST,
                "invalid_account",
                "Provide enabled",
            )
        })?;
        let mut disabled = row["disabled_key_positions"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        disabled.retain(|position| position.as_u64() != Some(input.position as u64));
        if !enabled {
            disabled.push(json!(input.position));
        }
        row["disabled_key_positions"] = json!(disabled);
    }
    state
        .config_publisher
        .apply_patch_revision(&json!({"providers":config["providers"]}), &revision)
        .await
        .map_err(management::configuration_error)?;
    state
        .runtime
        .refresh()
        .await
        .map_err(|_| management::unavailable())?;
    Ok(json!({"ok":true,"persistence":"source_file"}))
}

async fn import_accounts(state: &AppState, value: &Value, engine: &str) -> ApiResultKeys {
    let pairs = import_keys_with_metadata(value, engine)
        .map_err(|message| error(StatusCode::BAD_REQUEST, "invalid_import", &message))?;
    let mut keys = Vec::with_capacity(pairs.len());
    for (key, metadata) in pairs {
        if let Some(entry) = metadata {
            state
                .codex_oauth
                .record_imported_metadata(&key, entry)
                .await
                .map_err(|message| {
                    error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "oauth_import_failed",
                        &message,
                    )
                })?;
        }
        keys.push(key);
    }
    Ok(keys)
}

type ApiResultKeys = Result<Vec<String>, Response<Body>>;

#[cfg(test)]
pub(crate) fn import_keys(value: &Value, engine: &str) -> Result<Vec<String>, String> {
    Ok(import_keys_with_metadata(value, engine)?
        .into_iter()
        .map(|(key, _)| key)
        .collect())
}

fn import_keys_with_metadata<'a>(
    value: &'a Value,
    engine: &str,
) -> Result<Vec<(String, Option<&'a Value>)>, String> {
    let entries = value
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_else(|| std::slice::from_ref(value));
    let mut keys = Vec::new();
    for entry in entries {
        let (key, meta) = if let Some(key) = entry.as_str() {
            (key.trim().to_owned(), None)
        } else if engine == "codex" {
            let tokens = entry.get("tokens").unwrap_or(entry);
            if let Some(refresh) = tokens["refresh_token"]
                .as_str()
                .map(str::trim)
                .filter(|token| !token.is_empty())
            {
                let jwt_meta = tokens["id_token"]
                    .as_str()
                    .or_else(|| tokens["access_token"].as_str())
                    .map(crate::providers::codex::oauth::parse_jwt_metadata)
                    .unwrap_or_default();
                let account = tokens["account_id"]
                    .as_str()
                    .filter(|v| !v.trim().is_empty())
                    .or_else(|| {
                        entry["account_id"]
                            .as_str()
                            .filter(|value| !value.trim().is_empty())
                    })
                    .or(jwt_meta.account_id.as_deref())
                    .unwrap_or_default();
                if account.contains(',') {
                    return Err("Invalid account_id".into());
                }
                (format!("{account},{refresh}"), Some(entry))
            } else if let Some(access) = tokens["access_token"]
                .as_str()
                .or_else(|| entry["api_key"].as_str())
                .map(str::trim)
                .filter(|token| !token.is_empty())
            {
                (access.to_owned(), Some(entry))
            } else {
                return Err("Codex account requires refresh_token or access_token".into());
            }
        } else {
            let tokens = entry
                .get("claudeAiOauth")
                .or_else(|| entry.get("tokens"))
                .unwrap_or(entry);
            if let Some(refresh) = tokens
                .get("refresh_token")
                .or_else(|| tokens.get("refreshToken"))
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .filter(|_| matches!(engine, "claude" | "antigravity"))
            {
                (format!("oauth:{engine},{}", refresh.trim()), Some(entry))
            } else {
                let oauth_token = entry
                    .pointer("/claudeAiOauth/accessToken")
                    .or_else(|| entry.get("access_token"))
                    .or_else(|| entry.get("api_key"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|token| !token.is_empty())
                    .ok_or(
                        "Account requires api_key, access_token or a supported OAuth refresh_token",
                    )?;
                (oauth_token.to_owned(), Some(entry))
            }
        };
        if key.is_empty() || key.len() > 16384 || key.chars().any(char::is_control) {
            return Err(
                "Credentials must be nonempty, bounded and contain no control characters".into(),
            );
        }
        keys.push((key, meta));
    }
    if keys.is_empty() {
        return Err("Import is empty".into());
    }
    Ok(keys)
}

#[cfg(test)]
mod gateway_features {
    use super::*;
    #[test]
    fn account_import_accepts_codex_files_and_plain_key_batches_without_fake_oauth() {
        assert_eq!(import_keys(&json!({"tokens":{"account_id":"account","refresh_token":"refresh-secret","access_token":"not-returned"}}),"codex").unwrap(),vec!["account,refresh-secret"]);
        assert_eq!(
            import_keys(&json!(["first",{"api_key":"second"}]), "gemini").unwrap(),
            vec!["first", "second"]
        );
        assert_eq!(
            import_keys(&json!({"tokens":{"refresh_token":"secret"}}), "claude").unwrap(),
            vec!["oauth:claude,secret"]
        );
        assert!(import_keys(&json!(["bad\nkey"]), "gpt").is_err());
    }
}
