//! Narrow management surface using verified account sessions, never API-key impersonation.
use super::{bounded_body, channel_preview, error, response};
use crate::config::source::RuntimeConfigSource;
use crate::runtime::context::AppState;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{Method, Response, StatusCode, Uri};
use serde::Deserialize;
use serde_json::{json, Value};

type ApiResult = Result<Value, Response<Body>>;

pub(super) async fn handle(state: &AppState, request: Request, path: &str) -> Response<Body> {
    let method = request.method().clone();
    let result = match (method, path) {
        (Method::GET, "/admin/api/virtual-models") => super::virtual_models::read(state).await,
        (Method::PUT, "/admin/api/virtual-models") => {
            super::virtual_models::write(state, request).await
        }
        (Method::GET, "/admin/api/channels/keys") => {
            super::accounts::list(state, request.uri()).await
        }
        (Method::POST | Method::PATCH, "/admin/api/channels/keys") => {
            super::accounts::change(state, request).await
        }
        (Method::GET, "/admin/api/overview") => overview(state).await,
        (Method::GET, "/admin/api/update") => check_update(state).await,
        (Method::POST, "/admin/api/update") => apply_update(state).await,
        (Method::GET, "/admin/api/channels") => channels(state).await,
        (Method::GET, "/admin/api/channels/balances") => {
            super::balances::read(state, request.uri()).await
        }
        (Method::POST, "/admin/api/channels/balances") => {
            super::balances::refresh(state, request).await
        }
        (Method::GET, "/admin/api/api-keys") => api_keys(state).await,
        (Method::GET, "/admin/api/backend-logs") => super::backend_logs::read(request.uri()),
        (Method::GET, "/admin/api/workspace") => super::workspace::read(state, request.uri()).await,
        (Method::PUT, "/admin/api/workspace") => super::workspace::write(state, request).await,
        (Method::GET, "/admin/api/test/models") => {
            super::testing::models(state, request.uri()).await
        }
        (Method::POST, "/admin/api/test") => super::testing::run(state, request).await,
        (Method::GET, "/admin/api/logs") => Ok(state.admin.audit_logs()),
        (Method::GET, "/admin/api/usage") => usage(state, request.uri()).await,
        (Method::GET, "/admin/api/settings/schema") => Ok(crate::control::settings::schema()),
        (Method::GET, "/admin/api/channels/detail") => channel_detail(state, request.uri()).await,
        (Method::POST, "/admin/api/channels/models") => channel_models(state, request).await,
        (Method::POST, "/admin/api/channels/parameter-schema") => {
            channel_preview::handle(state, request, true).await
        }
        (Method::POST, "/admin/api/channels/preview") => {
            channel_preview::handle(state, request, false).await
        }
        (Method::GET, "/admin/api/settings") => {
            let provider = query(request.uri(), "provider")
                .unwrap_or_else(|| crate::control::global_settings::SCOPE.into());
            if provider == crate::control::global_settings::SCOPE {
                system_settings_view(state).await
            } else {
                state
                    .runtime
                    .settings_view(&provider)
                    .await
                    .map(|value| sanitize_for_state(state, value))
                    .map_err(settings_error)
            }
        }
        (Method::PATCH, "/admin/api/settings") | (Method::POST, "/admin/api/settings/validate") => {
            settings_change(state, request, path == "/admin/api/settings").await
        }
        (Method::POST | Method::PATCH | Method::DELETE, "/admin/api/channels") => {
            channel_change(state, request).await
        }
        (Method::POST | Method::PATCH | Method::DELETE, "/admin/api/api-keys") => {
            key_change(state, request).await
        }
        (
            _,
            "/admin/api/overview"
            | "/admin/api/update"
            | "/admin/api/channels"
            | "/admin/api/channels/balances"
            | "/admin/api/virtual-models"
            | "/admin/api/channels/keys"
            | "/admin/api/channels/detail"
            | "/admin/api/channels/models"
            | "/admin/api/channels/parameter-schema"
            | "/admin/api/channels/preview"
            | "/admin/api/api-keys"
            | "/admin/api/backend-logs"
            | "/admin/api/workspace"
            | "/admin/api/test"
            | "/admin/api/test/models"
            | "/admin/api/logs"
            | "/admin/api/usage"
            | "/admin/api/settings"
            | "/admin/api/settings/schema"
            | "/admin/api/settings/validate",
        ) => Err(error(
            StatusCode::METHOD_NOT_ALLOWED,
            "method_not_allowed",
            "Method not supported for this management resource",
        )),
        _ => Err(error(
            StatusCode::NOT_FOUND,
            "not_found",
            "Unknown management endpoint",
        )),
    };
    match result {
        Ok(value) => response(StatusCode::OK, value),
        Err(out) => out,
    }
}

async fn overview(state: &AppState) -> ApiResult {
    let (keys, providers, models) = state.runtime.runtime_counts().await;
    let stats = if state.persistence.disabled() {
        Value::Null
    } else {
        state
            .persistence
            .stats_summary(24)
            .await
            .unwrap_or(Value::Null)
    };
    Ok(json!({
        "runtime":"rust", "version":env!("CARGO_PKG_VERSION"),
        "configuration_ready":state.runtime.is_ready().await,
        "database_disabled":state.persistence.disabled(),
        "api_key_count":keys,"provider_count":providers,"model_count":models,
        "stats":sanitize(stats),"runtime_details":state.runtime_observability().await,
        "capabilities":{"channels_write":matches!(&state.config_publisher.source, RuntimeConfigSource::File(_)),"api_keys_write":matches!(&state.config_publisher.source, RuntimeConfigSource::File(_)),"settings_write":matches!(&state.config_publisher.source, RuntimeConfigSource::File(_)),"request_logs":false,"admin_audit":true,"plugins":true}
    }))
}

fn run_git_cmd(args: &[&str]) -> Option<String> {
    let mut cmd = std::process::Command::new("git");
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let out = cmd.output().ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        None
    }
}

async fn check_update(state: &AppState) -> ApiResult {
    let version = env!("CARGO_PKG_VERSION");
    let git_available = std::path::Path::new(".git").exists();
    let in_docker = std::path::Path::new("/.dockerenv").exists();
    let current_commit = if git_available {
        tokio::task::spawn_blocking(|| run_git_cmd(&["rev-parse", "HEAD"]))
            .await
            .ok()
            .flatten()
    } else {
        None
    }
    .or_else(|| option_env!("SOURCE_COMMIT").map(str::to_owned))
    .unwrap_or_else(|| "unknown".into());

    let mut latest_commit = String::new();
    let mut latest_message = String::new();
    let mut latest_date = String::new();
    let mut check_error = String::new();

    match state
        .backend_client
        .get("https://api.github.com/repos/Mitasul/miora/commits/main")
        .header("User-Agent", "miora-gateway")
        .header("Accept", "application/vnd.github+json")
        .timeout(std::time::Duration::from_secs(8))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(body) = resp.json::<Value>().await {
                latest_commit = body["sha"].as_str().unwrap_or_default().to_owned();
                latest_message = body
                    .pointer("/commit/message")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                latest_date = body
                    .pointer("/commit/committer/date")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
            }
        }
        Ok(resp) => {
            check_error = format!("GitHub API 返回 HTTP {}", resp.status().as_u16());
        }
        Err(err) => {
            check_error = format!("无法连接 GitHub API: {err}");
        }
    }

    if latest_commit.is_empty() && git_available {
        if let Ok((sha, msg, date)) = tokio::task::spawn_blocking(|| {
            let _ = run_git_cmd(&["fetch", "origin", "main"]);
            let sha = run_git_cmd(&["rev-parse", "origin/main"]).unwrap_or_default();
            let msg = run_git_cmd(&["log", "-1", "--format=%s", "origin/main"]).unwrap_or_default();
            let date =
                run_git_cmd(&["log", "-1", "--format=%cI", "origin/main"]).unwrap_or_default();
            (sha, msg, date)
        })
        .await
        {
            if !sha.is_empty() {
                latest_commit = sha;
                latest_message = msg;
                latest_date = date;
                check_error.clear();
            }
        }
    }

    let has_update = !latest_commit.is_empty()
        && !current_commit.is_empty()
        && current_commit != "unknown"
        && !latest_commit.starts_with(&current_commit)
        && !current_commit.starts_with(&latest_commit);

    Ok(json!({
        "version": version,
        "repo": "Mitasul/miora",
        "current_commit": current_commit,
        "latest_commit": latest_commit,
        "latest_message": latest_message,
        "latest_date": latest_date,
        "has_update": has_update,
        "git_available": git_available,
        "in_docker": in_docker,
        "check_error": check_error,
        "docker_command": "docker compose pull && docker compose up -d",
    }))
}

async fn apply_update(_state: &AppState) -> ApiResult {
    let git_available = std::path::Path::new(".git").exists();
    let in_docker = std::path::Path::new("/.dockerenv").exists();
    if in_docker || !git_available {
        return Ok(json!({
            "ok": false,
            "in_docker": true,
            "docker_command": "docker compose pull && docker compose up -d",
            "message": "当前为 Docker 容器或独立二进制部署，请在宿主机执行：docker compose pull && docker compose up -d"
        }));
    }
    let result = tokio::task::spawn_blocking(|| {
        let mut cmd = std::process::Command::new("git");
        cmd.args(["pull", "--ff-only", "origin", "main"]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        match cmd.output() {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout).trim().to_owned();
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
                let combined = if stderr.is_empty() {
                    stdout
                } else if stdout.is_empty() {
                    stderr
                } else {
                    format!("{stdout}\n{stderr}")
                };
                let head = run_git_cmd(&["rev-parse", "HEAD"]).unwrap_or_default();
                (out.status.success(), combined, head)
            }
            Err(err) => (false, format!("执行 git pull 失败: {err}"), String::new()),
        }
    })
    .await
    .map_err(|_| unavailable())?;

    let (ok, output, current_commit) = result;
    Ok(json!({
        "ok": ok,
        "output": output,
        "current_commit": current_commit,
        "message": if ok { "Git 仓库已拉取最新代码（如修改了后端源码请重新编译或拉取最新镜像重启）" } else { "拉取更新未成功，请检查输出信息" }
    }))
}

async fn channels(state: &AppState) -> ApiResult {
    if matches!(&state.config_publisher.source, RuntimeConfigSource::File(_)) {
        let (revision, config) = source_config(state).await?;
        let rows = config.get("providers").and_then(Value::as_array).into_iter().flatten().filter_map(|source| {
            let provider = crate::config::compiler::compile_provider(source)?;
            let mut models: Vec<_> = provider["models"].as_object()?.keys().cloned().collect();
            models.sort();
            Some(json!({"provider":provider["name"],"enabled":source.get("enabled").and_then(Value::as_bool).unwrap_or(true),"base_url":safe_url(provider["base_url"].as_str().unwrap_or_default()),"engine":provider["engine"],"channel_plugin":provider.pointer("/preferences/channel_plugin"),"groups":source.get("groups").or_else(|| source.pointer("/preferences/groups")).cloned().unwrap_or_else(|| json!(["default"])),"models":models,"api_key_count":crate::config::snapshot::provider_api_keys(&provider["api"]).len(),"temporary":false}))
        }).collect::<Vec<_>>();
        let groups = configured_key_groups(&config);
        return Ok(
            json!({"revision":revision,"channels":rows,"groups":groups,"writable":true,"secrets":"write_only","view":"source"}),
        );
    }
    let snapshot = state.runtime.snapshot().await.ok_or_else(unavailable)?;
    let controls = state.runtime.channel_controls.read().await;
    let channels = snapshot.providers.iter().map(|provider| {
        let mut models: Vec<_> = provider.models.keys().cloned().collect();
        models.sort();
        json!({"provider":provider.name.as_ref(),"base_url":safe_url(&provider.base_url),"engine":provider.engine.as_ref(),"channel_plugin":provider.preferences.get("channel_plugin"),"groups":crate::routing::catalog::groups(&provider.preferences),"models":models,"api_key_count":provider.api_keys.len(),"temporary":controls.temporary.contains_key(provider.name.as_ref())})
    }).collect::<Vec<_>>();
    let groups = configured_key_groups(&snapshot.api_config);
    Ok(
        json!({"revision":snapshot.revision.as_ref(),"channels":channels,"groups":groups,"writable":false,"secrets":"write_only","view":"runtime"}),
    )
}

#[cfg(test)]
fn compile_source(
    config: &Value,
    database_disabled: bool,
) -> Result<crate::config::snapshot::RawSnapshot, Response<Body>> {
    let raw = serde_json::to_vec(config).map_err(|_| unavailable())?;
    // Pure compilation: no model discovery, network request or source publication.
    let compiled = crate::config::compiler::compile_snapshot_bytes(&raw, database_disabled)
        .map_err(|_| unavailable())?;
    serde_json::from_slice(&compiled).map_err(|_| unavailable())
}

async fn channel_detail(state: &AppState, uri: &Uri) -> ApiResult {
    let provider = query(uri, "provider").unwrap_or_default();
    let (revision, config, writable) =
        if matches!(&state.config_publisher.source, RuntimeConfigSource::File(_)) {
            let (revision, config) = source_config(state).await?;
            (revision, config, true)
        } else {
            let snapshot = state.runtime.snapshot().await.ok_or_else(unavailable)?;
            (
                snapshot.revision.to_string(),
                snapshot.api_config.as_ref().clone(),
                false,
            )
        };
    let document = config
        .get("providers")
        .and_then(Value::as_array)
        .and_then(|rows| {
            rows.iter()
                .find(|row| row.get("provider").and_then(Value::as_str) == Some(provider.as_str()))
        })
        .ok_or_else(|| {
            error(
                StatusCode::NOT_FOUND,
                "channel_not_found",
                "Configured channel not found",
            )
        })?;
    let clean = sanitize_for_state(state, document.clone());
    Ok(
        json!({"provider":provider,"revision":revision,"base":clean,"effective":clean,"schema":crate::control::settings::schema(),"writable":writable,"persistence":"source_file","view":"source"}),
    )
}

#[derive(Deserialize)]
struct ModelDiscovery {
    provider: Option<String>,
    channel: DiscoveryChannel,
}

#[derive(Default, Deserialize)]
struct DiscoveryChannel {
    provider: Option<String>,
    engine: Option<String>,
    base_url: Option<String>,
    api: Option<Value>,
    channel_plugin: Option<String>,
    plugin_config: Option<serde_json::Map<String, Value>>,
}

async fn channel_models(state: &AppState, request: Request) -> ApiResult {
    let raw = bounded_body(request.into_body(), 262_144).await?;
    let input: ModelDiscovery = serde_json::from_slice(&raw).map_err(|_| {
        error(
            StatusCode::BAD_REQUEST,
            "invalid_model_discovery",
            "Expected an optional provider name and a channel object",
        )
    })?;
    let saved = if let Some(name) = input.provider {
        let config = match &state.config_publisher.source {
            RuntimeConfigSource::File(_) => source_config(state).await?.1,
            RuntimeConfigSource::Url(_) => state
                .runtime
                .snapshot()
                .await
                .ok_or_else(unavailable)?
                .api_config
                .as_ref()
                .clone(),
        };
        config
            .get("providers")
            .and_then(Value::as_array)
            .and_then(|rows| {
                rows.iter()
                    .find(|row| row.get("provider").and_then(Value::as_str) == Some(name.as_str()))
            })
            .cloned()
            .ok_or_else(|| {
                error(
                    StatusCode::NOT_FOUND,
                    "channel_not_found",
                    "Saved channel not found",
                )
            })?
    } else {
        json!({})
    };
    let mut channel = discovery_channel(saved, input.channel);
    crate::config::compiler::expand_environment(&mut channel);
    let engine = channel
        .get("engine")
        .and_then(Value::as_str)
        .filter(|engine| !engine.is_empty())
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| {
            crate::config::compiler::infer_engine(
                channel
                    .get("base_url")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            )
        });
    let plugin_id = channel
        .get("channel_plugin")
        .and_then(Value::as_str)
        .or_else(|| engine.strip_prefix("plugin:"));
    if let Some(id) = plugin_id {
        // models() merges registry defaults beneath these saved + draft schema values.
        let config = channel
            .get("plugin_config")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let key = crate::config::discovery::first_provider_key(channel.get("api"));
        let result = state
            .plugins
            .models_authenticated(
                &state.codex_oauth,
                id,
                channel
                    .get("provider")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                config,
                &key,
                channel
                    .pointer("/preferences/proxy")
                    .and_then(Value::as_str),
            )
            .await
            .map_err(|message| {
                error(StatusCode::BAD_GATEWAY, "model_discovery_failed", &message)
            })?;
        return Ok(json!({"models":result["models"],"source":"plugin"}));
    }
    if engine == "codex" {
        return Ok(json!({"models":codex_catalog_models(),"source":"catalog"}));
    }
    if engine == "azure" {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "model_discovery_unsupported",
            "Azure has no compatible model catalog here; enter deployment names manually",
        ));
    }
    channel["engine"] = json!(engine);
    let proxy = channel
        .pointer("/preferences/proxy")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let raw_key = crate::config::discovery::first_provider_key(channel.get("api"));
    if (matches!(engine.as_str(), "codex" | "claude" | "antigravity") && raw_key.contains(','))
        || raw_key.trim_start().starts_with('{')
    {
        if let Ok(auth) = state.codex_oauth.resolve(&raw_key, proxy.as_deref()).await {
            channel["api"] = json!(auth.bearer);
        }
    }
    let client = state
        .upstream_client(proxy.as_deref(), true, None)
        .await
        .map_err(|_| {
            error(
                StatusCode::BAD_REQUEST,
                "model_discovery_failed",
                "Could not initialize the model discovery HTTP client",
            )
        })?;
    let ids = crate::config::discovery::discover_provider_models(&client, &channel)
        .await
        .map_err(|message| error(StatusCode::BAD_GATEWAY, "model_discovery_failed", &message))?;
    Ok(
        json!({"models":ids.into_iter().map(|id| json!({"id":id})).collect::<Vec<_>>(),"source":"upstream"}),
    )
}

fn discovery_channel(saved: Value, overrides: DiscoveryChannel) -> Value {
    let mut channel = saved.as_object().cloned().unwrap_or_default();
    if let Some(name) = overrides.provider.filter(|name| !name.is_empty()) {
        channel.insert("provider".into(), json!(name));
    }
    // Source configs also allow plugin fields beneath preferences.
    for field in ["channel_plugin", "plugin_config"] {
        if !channel.contains_key(field) {
            if let Some(value) = saved
                .get("preferences")
                .and_then(|preferences| preferences.get(field))
            {
                channel.insert(field.into(), value.clone());
            }
        }
    }
    if let Some(engine) = overrides.engine {
        channel.insert("engine".into(), json!(engine));
    }
    if let Some(base_url) = overrides.base_url {
        channel.insert("base_url".into(), json!(base_url));
    }
    if let Some(api) = overrides
        .api
        .filter(|api| !api.is_null() && !api.as_str().is_some_and(|key| key.trim().is_empty()))
    {
        channel.insert("api".into(), api);
    }
    if let Some(id) = overrides.channel_plugin {
        channel.insert("channel_plugin".into(), json!(id));
    }
    if let Some(overrides) = overrides.plugin_config {
        let mut config = channel
            .get("plugin_config")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        config.extend(overrides);
        channel.insert("plugin_config".into(), Value::Object(config));
    }
    Value::Object(channel)
}

fn codex_catalog_models() -> Vec<Value> {
    let catalog: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/codex/codex_models_pro_0_153_2.json"
    )))
    .expect("valid bundled Codex catalog");
    catalog["models"]
        .as_array()
        .expect("bundled Codex model array")
        .iter()
        .map(|model| json!({"id":model["slug"],"name":model["display_name"]}))
        .collect()
}

fn configured_key_groups(config: &Value) -> Vec<String> {
    config
        .pointer("/preferences/key_groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

async fn api_keys(state: &AppState) -> ApiResult {
    if matches!(&state.config_publisher.source, RuntimeConfigSource::File(_)) {
        let (revision, config) = source_config(state).await?;
        let rows = key_rows(&config);
        let groups = configured_key_groups(&config);
        return Ok(
            json!({"revision":revision,"api_keys":rows,"groups":groups,"writable":true,"read_only":false,"view":"source"}),
        );
    }
    let snapshot = state.runtime.snapshot().await.ok_or_else(unavailable)?;
    let rows = key_rows(&snapshot.api_config);
    let groups = configured_key_groups(&snapshot.api_config);
    Ok(
        json!({"revision":snapshot.revision.as_ref(),"api_keys":rows,"groups":groups,"writable":false,"read_only":true,"view":"runtime"}),
    )
}

fn key_rows(config: &Value) -> Vec<Value> {
    config.get("api_keys").and_then(Value::as_array).into_iter().flatten()
        .enumerate().filter_map(|(index, key)| {
            let token = key.get("api")?.as_str()?;
            Some(json!({"id":crate::routing::catalog::key_id(token),"position":index+1,
                "name":key.get("name").or_else(|| key.pointer("/preferences/name")).and_then(Value::as_str).unwrap_or(""),
                "groups":key.get("groups").or_else(|| key.pointer("/preferences/groups")).cloned().unwrap_or_else(|| json!(["default"])),
                "enabled":key.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                "role":key.get("role").and_then(Value::as_str).unwrap_or("user"),
                "models":key.get("model").cloned().unwrap_or_else(|| json!(["all"])),
                "rate_limit":key.pointer("/preferences/rate_limit").cloned().unwrap_or(Value::Null),
                "masked_key":"********"}))
        }).collect()
}

async fn usage(state: &AppState, uri: &Uri) -> ApiResult {
    if state.persistence.disabled() {
        return Ok(json!({"usage":[],"available":false,"reason":"database_disabled"}));
    }
    let days = query(uri, "days")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(30)
        .clamp(1, 365);
    let end = super::auth::unix_seconds().min(i64::MAX as u64) as i64;
    let start = end.saturating_sub((days * 86_400) as i64);
    let model = query(uri, "model");
    state
        .persistence
        .model_usage(
            start,
            end,
            model.as_deref(),
            query(uri, "channel").as_deref(),
        )
        .await
        .map_err(|_| {
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "statistics_unavailable",
                "Usage statistics unavailable",
            )
        })
}

async fn system_settings_view(state: &AppState) -> ApiResult {
    use crate::control::global_settings;
    let writable = matches!(&state.config_publisher.source, RuntimeConfigSource::File(_));
    let source = if writable {
        Some(source_config(state).await?)
    } else {
        None
    };
    let controls = state.runtime.channel_controls.read().await;
    let base = state
        .runtime
        .base_snapshot()
        .await
        .ok_or_else(unavailable)?;
    let (source_revision, config) =
        source.unwrap_or_else(|| (base.revision.to_string(), base.api_config.as_ref().clone()));
    let preferences = config
        .get("preferences")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let preferences = preferences.as_object().ok_or_else(unavailable)?;
    let raw = global_settings::document(preferences);
    let effective = global_settings::document(&global_settings::overlay(
        preferences,
        controls.settings.get(global_settings::SCOPE),
    ));
    let revision = format!("{source_revision}:{}", controls.revision(&base));
    Ok(sanitize_for_state(
        state,
        json!({
            "provider":global_settings::SCOPE,"kind":"global","revision":revision,
            "base":raw,"effective":effective,"resolved_fields":global_settings::resolved_fields(&effective),
            "schema":global_settings::schema(),"writable":writable,
            "persistence":if writable { "source_file" } else { "read_only" }
        }),
    ))
}

async fn system_settings_change(
    state: &AppState,
    input: crate::control::settings::Mutation,
    apply: bool,
) -> ApiResult {
    use crate::control::{
        global_settings,
        settings::{merge, ProviderSettings},
    };
    let invalid = || settings_error((StatusCode::BAD_REQUEST, String::new()));
    if input.changes.len() != 1 || input.operation_id.is_empty() || input.operation_id.len() > 128 {
        return Err(invalid());
    }
    let change = &input.changes[0];
    if change.provider != global_settings::SCOPE
        || !change.copy_to_key.is_empty()
        || !change.create_to_key.is_empty()
        || change.delete_copy
    {
        return Err(invalid());
    }
    global_settings::validate_intent(&ProviderSettings {
        set: change.set.clone(),
        remove: change.remove.clone(),
        compiled: None,
    })
    .map_err(|_| invalid())?;
    let _guard = state.admin.config_lock().try_lock().map_err(|_| {
        error(
            StatusCode::CONFLICT,
            "configuration_busy",
            "Another admin configuration write is in progress",
        )
    })?;
    let (source_revision, config) = source_config(state).await?;
    let mut controls = state.runtime.channel_controls.write().await;
    let fingerprint = serde_json::to_string(&input.changes).unwrap_or_default();
    if apply {
        if let Some((old, result)) = controls.settings_operations.get(&input.operation_id) {
            return if old == &fingerprint && result["persistence"] == "source_file" {
                Ok(result.as_ref().clone())
            } else {
                Err(settings_error((StatusCode::CONFLICT, String::new())))
            };
        }
    }
    let base = state
        .runtime
        .base_snapshot()
        .await
        .ok_or_else(unavailable)?;
    if input.revision != format!("{source_revision}:{}", controls.revision(&base)) {
        return Err(settings_error((StatusCode::CONFLICT, String::new())));
    }
    let mut preferences = config
        .get("preferences")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let preferences = preferences.as_object_mut().ok_or_else(unavailable)?;
    let raw = global_settings::document(preferences);
    let before = global_settings::document(&global_settings::overlay(
        preferences,
        controls.settings.get(global_settings::SCOPE),
    ));
    let after = merge(
        if change.reset { &raw } else { &before },
        &change.set,
        &change.remove,
    );
    global_settings::validate(&after).map_err(|_| invalid())?;
    let mut result = json!({"operation_id":input.operation_id,"status":if apply{"applied"}else{"validated"},"revision":input.revision,"previews":[{"provider":global_settings::SCOPE,"before":before,"after":after}],"persistence":"source_file"});
    if apply {
        for key in global_settings::KEYS {
            if let Some(value) = after["preferences"].get(*key) {
                preferences.insert((*key).into(), value.clone());
            } else {
                preferences.remove(*key);
            }
        }
        state
            .config_publisher
            .apply_patch_revision(&json!({"preferences":preferences}), &source_revision)
            .await
            .map_err(configuration_error)?;
        state.runtime.refresh().await.map_err(|_| {
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "runtime_refresh_failed",
                "Configuration saved but runtime refresh failed",
            )
        })?;
        controls.settings.remove(global_settings::SCOPE);
        controls.sequence += 1;
        let base = state
            .runtime
            .base_snapshot()
            .await
            .ok_or_else(unavailable)?;
        let revision = self::source_revision(state).await?;
        result["revision"] = json!(format!("{revision}:{}", controls.revision(&base)));
        controls.settings_operations.insert(
            input.operation_id,
            (fingerprint, std::sync::Arc::new(result.clone())),
        );
        if controls.settings_operations.len() > 256 {
            if let Some(key) = controls.settings_operations.keys().next().cloned() {
                controls.settings_operations.remove(&key);
            }
        }
    }
    Ok(sanitize_for_state(state, result))
}

async fn settings_change(state: &AppState, request: Request, apply: bool) -> ApiResult {
    let raw = bounded_body(request.into_body(), 262_144).await?;
    let input =
        serde_json::from_slice::<crate::control::settings::Mutation>(&raw).map_err(|_| {
            error(
                StatusCode::BAD_REQUEST,
                "invalid_settings",
                "Expected revision, operation_id and valid changes",
            )
        })?;
    if input.changes.len() > 100 {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "too_many_changes",
            "At most 100 settings changes are permitted",
        ));
    }
    if input
        .changes
        .iter()
        .any(|change| change.provider == crate::control::global_settings::SCOPE)
    {
        return system_settings_change(state, input, apply).await;
    }
    let value = state
        .runtime
        .settings_change(input, apply)
        .await
        .map_err(settings_error)?;
    let mut value = sanitize_for_state(state, value);
    value["persistence"] = json!("runtime_only");
    Ok(value)
}

fn settings_error((status, _message): (StatusCode, String)) -> Response<Body> {
    let (code, message) = match status {
        StatusCode::CONFLICT => (
            "revision_conflict",
            "Configuration changed or operation_id was reused; refresh before retrying",
        ),
        StatusCode::NOT_FOUND => ("not_found", "Channel or settings operation not found"),
        StatusCode::SERVICE_UNAVAILABLE => (
            "configuration_unavailable",
            "Runtime configuration unavailable",
        ),
        _ => (
            "invalid_settings",
            "Settings validation failed; check values against the settings schema",
        ),
    };
    // Compiler errors may include secret input values, so do not relay arbitrary error strings.
    error(status, code, message)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChannelMutation {
    revision: String,
    #[serde(default)]
    provider: String,
    #[serde(default)]
    providers: Vec<String>,
    #[serde(default)]
    copy_from: String,
    #[serde(default)]
    channel: Value,
    #[serde(default)]
    patch: Value,
}

pub(super) async fn source_revision(state: &AppState) -> Result<String, Response<Body>> {
    let RuntimeConfigSource::File(path) = &state.config_publisher.source else {
        return Err(error(
            StatusCode::FORBIDDEN,
            "configuration_read_only",
            "Remote CONFIG_URL is read-only",
        ));
    };
    let metadata = tokio::fs::metadata(path).await.map_err(|_| unavailable())?;
    crate::config::source::file_revision(&metadata).map_err(|_| unavailable())
}

pub(super) async fn source_config(state: &AppState) -> Result<(String, Value), Response<Body>> {
    let RuntimeConfigSource::File(path) = &state.config_publisher.source else {
        return Err(error(
            StatusCode::FORBIDDEN,
            "configuration_read_only",
            "Remote CONFIG_URL is read-only",
        ));
    };
    let metadata = tokio::fs::metadata(path).await.map_err(|_| unavailable())?;
    if metadata.len() > 4 * 1024 * 1024 {
        return Err(error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "configuration_too_large",
            "Admin configuration editing supports files up to 4 MiB",
        ));
    }
    let raw = tokio::fs::read(path).await.map_err(|_| unavailable())?;
    let config = serde_yaml::from_slice::<Value>(&raw).map_err(|_| unavailable())?;
    let revision = crate::config::source::file_revision(&metadata).map_err(|_| unavailable())?;
    Ok((revision, config))
}

async fn channel_change(state: &AppState, request: Request) -> ApiResult {
    let method = request.method().clone();
    let raw = bounded_body(request.into_body(), 262_144).await?;
    let input = serde_json::from_slice::<ChannelMutation>(&raw).map_err(|_| {
        error(
            StatusCode::BAD_REQUEST,
            "invalid_channel",
            "Invalid channel mutation",
        )
    })?;
    // Separate config-only lock: never blocks session checks or password verification.
    let _guard = state.admin.config_lock().try_lock().map_err(|_| {
        error(
            StatusCode::CONFLICT,
            "configuration_busy",
            "Another admin configuration write is in progress",
        )
    })?;
    let (revision, config) = source_config(state).await?;
    if input.revision.is_empty() || input.revision != revision {
        return Err(error(
            StatusCode::CONFLICT,
            "revision_conflict",
            "Configuration changed; refresh channels before retrying",
        ));
    }
    let mut providers = config
        .get("providers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if method == Method::POST {
        let provider = input
            .channel
            .get("provider")
            .and_then(Value::as_str)
            .unwrap_or("");
        if provider.is_empty()
            || provider.len() > 128
            || provider.chars().any(char::is_control)
            || providers
                .iter()
                .any(|p| p.get("provider").and_then(Value::as_str) == Some(provider))
        {
            return Err(error(
                StatusCode::BAD_REQUEST,
                "invalid_channel",
                "Provider name must be unique and contain 1-128 non-control bytes",
            ));
        }
        let channel = if input.copy_from.is_empty() {
            let channel = channel_preview::merge_channel(&json!({}), &input.channel)
                .map_err(channel_preview::policy_error)?;
            validate_channel(&channel, true)?;
            validate_plugin_config(state, &channel)?;
            channel_preview::validate_document_policy(&channel)
                .map_err(channel_preview::policy_error)?;
            channel
        } else {
            let mut channel = providers
                .iter()
                .find(|row| row["provider"] == input.copy_from)
                .cloned()
                .ok_or_else(|| {
                    error(
                        StatusCode::NOT_FOUND,
                        "channel_not_found",
                        "Source channel not found",
                    )
                })?;
            channel["provider"] = provider.into();
            channel
        };
        providers.push(channel);
    } else if method == Method::PATCH && !input.providers.is_empty() {
        if input.providers.len() > 512
            || input.patch.as_object().is_none_or(|patch| {
                patch.is_empty()
                    || patch
                        .keys()
                        .any(|key| !matches!(key.as_str(), "enabled" | "groups"))
            })
        {
            return Err(error(
                StatusCode::BAD_REQUEST,
                "invalid_batch",
                "Batch changes accept enabled and groups for up to 512 channels",
            ));
        }
        for name in &input.providers {
            let channel = providers
                .iter_mut()
                .find(|row| row["provider"].as_str() == Some(name))
                .ok_or_else(|| {
                    error(
                        StatusCode::NOT_FOUND,
                        "channel_not_found",
                        "Selected channel not found",
                    )
                })?;
            let merged = channel_preview::merge_channel(channel, &input.patch)
                .map_err(channel_preview::policy_error)?;
            validate_channel(&merged, true)?;
            *channel = merged;
        }
    } else {
        let index = providers
            .iter()
            .position(|p| p.get("provider").and_then(Value::as_str) == Some(&input.provider))
            .ok_or_else(|| {
                error(
                    StatusCode::NOT_FOUND,
                    "channel_not_found",
                    "Configured channel not found; runtime-only channels cannot be edited here",
                )
            })?;
        if method == Method::DELETE {
            providers.remove(index);
        } else {
            if input.patch.get("provider").is_some() {
                return Err(error(
                    StatusCode::BAD_REQUEST,
                    "invalid_channel",
                    "Provider renaming is not supported",
                ));
            }
            let mut merged = channel_preview::merge_channel(&providers[index], &input.patch)
                .map_err(channel_preview::policy_error)?;
            if input.patch.get("api").is_some()
                && input.patch.get("disabled_key_positions").is_none()
                && input.patch["api"] != providers[index]["api"]
            {
                merged
                    .as_object_mut()
                    .expect("merged channel")
                    .remove("disabled_key_positions");
            }
            validate_channel(&merged, true)?;
            validate_plugin_config(state, &merged)?;
            channel_preview::validate_document_policy(&merged)
                .map_err(channel_preview::policy_error)?;
            providers[index] = merged;
        }
    }
    state
        .config_publisher
        .apply_patch_revision(&json!({"providers":providers}), &revision)
        .await
        .map_err(configuration_error)?;
    state.runtime.refresh().await.map_err(|_| {
        error(
            StatusCode::SERVICE_UNAVAILABLE,
            "runtime_refresh_failed",
            "Configuration saved but runtime refresh failed",
        )
    })?;
    let revision = source_revision(state).await?;
    Ok(json!({"ok":true,"revision":revision,"persistence":"source_file"}))
}

pub(super) fn configuration_error(message: String) -> Response<Body> {
    if message.contains("revision conflict") {
        error(
            StatusCode::CONFLICT,
            "revision_conflict",
            "Configuration changed; refresh before retrying",
        )
    } else if [
        "Virtual model",
        "Unknown virtual",
        "First target of virtual",
        "on_status belongs",
        "Virtual target",
        "Reasoning suffix",
    ]
    .iter()
    .any(|prefix| message.starts_with(prefix))
    {
        error(StatusCode::BAD_REQUEST, "invalid_configuration", &message)
    } else {
        error(StatusCode::BAD_REQUEST, "configuration_write_failed", "Configuration validation or publication failed; refresh and inspect server configuration")
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyMutation {
    revision: String,
    #[serde(default)]
    id: String,
    models: Option<Vec<String>>,
    role: Option<String>,
    name: Option<String>,
    groups: Option<Vec<String>>,
    enabled: Option<bool>,
    rate_limit: Option<String>,
    group_action: Option<String>,
    group_name: Option<String>,
}

async fn key_change(state: &AppState, request: Request) -> ApiResult {
    let method = request.method().clone();
    let raw = bounded_body(request.into_body(), 262_144).await?;
    let input = serde_json::from_slice::<KeyMutation>(&raw).map_err(|_| {
        error(
            StatusCode::BAD_REQUEST,
            "invalid_api_key",
            "Expected revision, optional id, models and role",
        )
    })?;
    if let Some(action) = input.group_action.as_deref() {
        let group_name = input.group_name.as_deref().unwrap_or("").trim().to_owned();
        validate_groups(&json!([group_name.clone()]))?;
        let _guard = state.admin.config_lock().try_lock().map_err(|_| {
            error(
                StatusCode::CONFLICT,
                "configuration_busy",
                "Another admin configuration write is in progress",
            )
        })?;
        let (revision, config) = source_config(state).await?;
        if input.revision.is_empty() || input.revision != revision {
            return Err(error(
                StatusCode::CONFLICT,
                "revision_conflict",
                "Configuration changed; refresh API keys before retrying",
            ));
        }
        let mut preferences = config
            .get("preferences")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let mut list = configured_key_groups(&config);
        match action {
            "add" => {
                for item in group_name
                    .split([',', '，'])
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    validate_groups(&json!([item]))?;
                    if !list.iter().any(|g| g == item) {
                        list.push(item.to_owned());
                    }
                }
            }
            "remove" => {
                list.retain(|g| g != &group_name);
            }
            _ => {
                return Err(error(
                    StatusCode::BAD_REQUEST,
                    "invalid_group_action",
                    "group_action must be add or remove",
                ))
            }
        }
        preferences.insert("key_groups".into(), json!(list));
        state
            .config_publisher
            .apply_patch_revision(&json!({"preferences":preferences}), &revision)
            .await
            .map_err(configuration_error)?;
        state.runtime.refresh().await.map_err(|_| {
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "runtime_refresh_failed",
                "Configuration saved but runtime refresh failed",
            )
        })?;
        let revision = source_revision(state).await?;
        return Ok(
            json!({"ok":true,"revision":revision,"groups":list,"persistence":"source_file"}),
        );
    }
    if let Some(groups) = &input.groups {
        validate_groups(&json!(groups))?;
    }
    if input
        .name
        .as_ref()
        .is_some_and(|name| name.len() > 128 || name.chars().any(char::is_control))
    {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "invalid_name",
            "Name must be at most 128 bytes without control characters",
        ));
    }
    if input.rate_limit.as_ref().is_some_and(|rate| {
        !rate.is_empty()
            && crate::runtime::scheduling::parse_rate_limits(Some(&json!(rate)), None).is_none()
    }) {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "invalid_rate_limit",
            "Use a rate such as 60/min, or leave it empty",
        ));
    }
    if input
        .role
        .as_deref()
        .is_some_and(|role| !matches!(role, "user" | "admin"))
    {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "invalid_role",
            "Role must be exactly user or admin",
        ));
    }
    if input.models.as_ref().is_some_and(|models| {
        models.is_empty()
            || models.len() > 4096
            || models.iter().any(|model| {
                model.is_empty() || model.len() > 512 || model.chars().any(char::is_control)
            })
    }) {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "invalid_models",
            "Models must be bounded nonempty strings",
        ));
    }
    let _guard = state.admin.config_lock().try_lock().map_err(|_| {
        error(
            StatusCode::CONFLICT,
            "configuration_busy",
            "Another admin configuration write is in progress",
        )
    })?;
    let (revision, config) = source_config(state).await?;
    if input.revision.is_empty() || input.revision != revision {
        return Err(error(
            StatusCode::CONFLICT,
            "revision_conflict",
            "Configuration changed; refresh API keys before retrying",
        ));
    }
    let mut keys = config
        .get("api_keys")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut revealed = None;
    let id = if method == Method::POST {
        if !input.id.is_empty() {
            return Err(error(
                StatusCode::BAD_REQUEST,
                "invalid_api_key",
                "New key ids are generated by the server",
            ));
        }
        let token = format!(
            "sk-{}",
            super::auth::random_token().map_err(|_| unavailable())?
        );
        let id = crate::routing::catalog::key_id(&token);
        let mut row = json!({"api":token,"role":input.role.clone().unwrap_or_else(|| "user".into()),"model":input.models.clone().unwrap_or_else(|| vec!["all".into()])});
        update_key_metadata(&mut row, &input);
        keys.push(row);
        revealed = Some(token);
        id
    } else {
        let index = keys
            .iter()
            .position(|key| {
                key.get("api")
                    .and_then(Value::as_str)
                    .is_some_and(|token| crate::routing::catalog::key_id(token) == input.id)
            })
            .ok_or_else(|| {
                error(
                    StatusCode::NOT_FOUND,
                    "api_key_not_found",
                    "API key not found",
                )
            })?;
        if method == Method::DELETE {
            keys.remove(index);
        } else {
            if input.models.is_none()
                && input.role.is_none()
                && input.name.is_none()
                && input.groups.is_none()
                && input.enabled.is_none()
                && input.rate_limit.is_none()
            {
                return Err(error(
                    StatusCode::BAD_REQUEST,
                    "invalid_api_key",
                    "Provide models or role to update",
                ));
            }
            let row = keys[index].as_object_mut().ok_or_else(unavailable)?;
            if let Some(models) = &input.models {
                row.insert("model".into(), json!(models));
            }
            if let Some(role) = &input.role {
                row.insert("role".into(), json!(role));
            }
            update_key_metadata(&mut keys[index], &input);
        }
        input.id
    };
    state
        .config_publisher
        .apply_patch_revision(&json!({"api_keys":keys}), &revision)
        .await
        .map_err(configuration_error)?;
    state.runtime.refresh().await.map_err(|_| {
        error(
            StatusCode::SERVICE_UNAVAILABLE,
            "runtime_refresh_failed",
            "Configuration saved but runtime refresh failed",
        )
    })?;
    let revision = source_revision(state).await?;
    let mut out = json!({"ok":true,"revision":revision,"id":id,"persistence":"source_file"});
    if let Some(token) = revealed {
        out["api_key"] = json!(token);
        out["show_once"] = json!(true);
    }
    Ok(out)
}

fn update_key_metadata(row: &mut Value, input: &KeyMutation) {
    if let Some(name) = &input.name {
        row["name"] = json!(name.trim());
    }
    if let Some(groups) = &input.groups {
        row["groups"] = json!(groups);
    }
    if let Some(enabled) = input.enabled {
        row["enabled"] = json!(enabled);
    }
    if let Some(rate) = &input.rate_limit {
        let preferences = row
            .as_object_mut()
            .expect("key object")
            .entry("preferences")
            .or_insert_with(|| json!({}));
        if !preferences.is_object() {
            *preferences = json!({});
        }
        if rate.is_empty() {
            preferences
                .as_object_mut()
                .expect("key preferences")
                .remove("rate_limit");
        } else {
            preferences["rate_limit"] = json!(rate);
        }
    }
}

pub(super) fn validate_groups(value: &Value) -> Result<(), Response<Body>> {
    let valid = crate::config::compiler::valid_groups(value);
    if valid {
        Ok(())
    } else {
        Err(error(
            StatusCode::BAD_REQUEST,
            "invalid_groups",
            "Groups must contain 1 to 32 nonempty names, at most 64 bytes each",
        ))
    }
}

fn validate_plugin_config(state: &AppState, document: &Value) -> Result<(), Response<Body>> {
    let id = document
        .get("channel_plugin")
        .or_else(|| document.pointer("/preferences/channel_plugin"));
    let Some(id) = id else {
        return Ok(());
    };
    let Some(id) = id.as_str().filter(|id| !id.is_empty()) else {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "invalid_plugin",
            "channel_plugin must name an enabled channel plugin",
        ));
    };
    let empty = serde_json::Map::new();
    let config = match document
        .get("plugin_config")
        .or_else(|| document.pointer("/preferences/plugin_config"))
    {
        Some(config) => config.as_object().ok_or_else(|| {
            error(
                StatusCode::BAD_REQUEST,
                "invalid_plugin_config",
                "plugin_config must be an object",
            )
        })?,
        None => &empty,
    };
    state
        .plugins
        .validate_channel_config(id, config)
        .map_err(|_| {
            error(
                StatusCode::BAD_REQUEST,
                "invalid_plugin_config",
                "Plugin missing, disabled, or configuration does not match its schema",
            )
        })
}

pub(super) fn sanitize_for_state(state: &AppState, value: Value) -> Value {
    fn plugin_redact(state: &AppState, value: Value) -> Value {
        match value {
            Value::Object(mut object) => {
                if let Some(config) = object.get("plugin_config").and_then(Value::as_object) {
                    let id = object
                        .get("channel_plugin")
                        .and_then(Value::as_str)
                        .or_else(|| {
                            object
                                .get("preferences")
                                .and_then(|p| p.get("channel_plugin"))
                                .and_then(Value::as_str)
                        })
                        .unwrap_or("");
                    let redacted = state.plugins.redact_channel_config(id, config);
                    object.insert("plugin_config".into(), Value::Object(redacted));
                }
                Value::Object(
                    object
                        .into_iter()
                        .map(|(key, value)| (key, plugin_redact(state, value)))
                        .collect(),
                )
            }
            Value::Array(values) => Value::Array(
                values
                    .into_iter()
                    .map(|value| plugin_redact(state, value))
                    .collect(),
            ),
            other => other,
        }
    }
    sanitize(plugin_redact(state, value))
}

fn validate_channel(value: &Value, create: bool) -> Result<(), Response<Body>> {
    if value
        .get("enabled")
        .is_some_and(|enabled| !enabled.is_boolean())
    {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "invalid_enabled",
            "enabled must be a boolean",
        ));
    }
    if let Some(groups) = value.get("groups") {
        validate_groups(groups)?;
    }
    if let Some(pricing) = value.pointer("/preferences/usage_pricing") {
        crate::storage::usage::validate_pricing(pricing)
            .map_err(|message| error(StatusCode::BAD_REQUEST, "invalid_pricing", &message))?;
    }
    if let Some(mode) = value.pointer("/preferences/protocol_mode") {
        if !matches!(mode.as_str(), Some("auto" | "native")) {
            return Err(error(
                StatusCode::BAD_REQUEST,
                "invalid_protocol_mode",
                "protocol_mode must be auto or native",
            ));
        }
    }
    let invalid = || {
        error(StatusCode::BAD_REQUEST, "invalid_channel", "Channel must use supported provider fields, valid HTTP(S) base_url and explicit model list; masked secret placeholders cannot be saved")
    };
    let Some(object) = value.as_object() else {
        return Err(invalid());
    };
    if object.is_empty() || (!create && object.contains_key("provider")) {
        return Err(invalid());
    }
    let plugin = value
        .get("channel_plugin")
        .or_else(|| value.pointer("/preferences/channel_plugin"))
        .and_then(Value::as_str)
        .is_some_and(|v| !v.is_empty());
    if value
        .pointer("/preferences/protocol_mode")
        .and_then(Value::as_str)
        == Some("native")
    {
        let engine = value
            .get("engine")
            .and_then(Value::as_str)
            .map(|value| value.trim().to_ascii_lowercase())
            .unwrap_or_else(|| {
                crate::config::compiler::infer_engine(
                    value
                        .get("base_url")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                )
            });
        if plugin
            || !matches!(
                engine.as_str(),
                "gpt"
                    | "openrouter"
                    | "codex"
                    | "antigravity"
                    | "claude"
                    | "gemini"
                    | "vertex"
                    | "vertex-gemini"
            )
        {
            return Err(error(
                StatusCode::BAD_REQUEST,
                "invalid_protocol_mode",
                "Native mode requires a supported native built-in protocol",
            ));
        }
    }
    if create && ((!plugin && !object.contains_key("base_url")) || !object.contains_key("model")) {
        return Err(invalid());
    }
    if let Some(url) = object
        .get("base_url")
        .filter(|v| !plugin || v.as_str().is_some_and(|v| !v.is_empty()))
    {
        let url = url
            .as_str()
            .and_then(|v| url::Url::parse(v).ok())
            .ok_or_else(invalid)?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(invalid());
        }
    }
    // Explicit models avoid save-time outbound model discovery / paid or implicit probes.
    if let Some(models) = object.get("model") {
        let Some(models) = models.as_array() else {
            return Err(invalid());
        };
        if models.is_empty()
            || models.len() > 4096
            || models
                .iter()
                .any(|v| v == "all" || (!v.is_string() && !v.is_object()))
        {
            return Err(invalid());
        }
    }
    if contains_mask(value) {
        return Err(invalid());
    }
    Ok(())
}

fn contains_mask(value: &Value) -> bool {
    match value {
        Value::String(v) => v == "***" || v == "********" || v == "[REDACTED]",
        Value::Object(v) => v.contains_key("$secret") || v.values().any(contains_mask),
        Value::Array(v) => v.iter().any(contains_mask),
        _ => false,
    }
}

fn query(uri: &Uri, name: &str) -> Option<String> {
    url::form_urlencoded::parse(uri.query()?.as_bytes())
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

pub(super) fn unavailable() -> Response<Body> {
    error(
        StatusCode::SERVICE_UNAVAILABLE,
        "configuration_unavailable",
        "Runtime configuration unavailable",
    )
}

pub(super) fn safe_url(value: &str) -> String {
    let Ok(mut url) = url::Url::parse(value) else {
        return "[invalid URL]".into();
    };
    if !matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h") {
        return "[redacted URL]".into();
    }
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    url.to_string()
}

fn opaque_reference(value: &Value) -> bool {
    match value {
        Value::Object(v) => v.len() == 1 && v.get("$secret").is_some_and(Value::is_string),
        Value::Array(v) => v.iter().all(opaque_reference),
        Value::Null => true,
        _ => false,
    }
}

/// Defense in depth on top of the runtime's opaque-reference redaction.
fn sanitize(value: Value) -> Value {
    match value {
        Value::Object(mut object) => {
            if object
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| {
                    crate::providers::parameters::header_sensitive(
                        name,
                        object.get("secret") == Some(&Value::Bool(true)),
                    )
                })
                && object.contains_key("value")
            {
                object.insert("value".into(), json!("********"));
            }
            Value::Object(
                object
                    .into_iter()
                    .map(|(key, value)| {
                        let normalized = key.to_ascii_lowercase().replace('-', "_");
                        let value = if normalized == "secret" && value.is_boolean() {
                            value
                        } else if normalized == "headers" || normalized == "http_headers" {
                            match value {
                                Value::Array(rows) => {
                                    Value::Array(rows.into_iter().map(sanitize).collect())
                                }
                                Value::Object(headers) => Value::Object(
                                    headers
                                        .into_iter()
                                        .map(|(name, value)| {
                                            let value =
                                                if crate::providers::parameters::header_sensitive(
                                                    &name, false,
                                                ) {
                                                    json!("********")
                                                } else {
                                                    sanitize(value)
                                                };
                                            (name, value)
                                        })
                                        .collect(),
                                ),
                                value => sanitize(value),
                            }
                        } else if matches!(
                            normalized.as_str(),
                            "api"
                                | "api_key"
                                | "api_keys"
                                | "token"
                                | "password"
                                | "secret"
                                | "private_key"
                                | "authorization"
                                | "cookie"
                                | "client_secret"
                                | "aws_access_key"
                                | "aws_secret_key"
                                | "aws_session_token"
                        ) {
                            if opaque_reference(&value) {
                                value
                            } else {
                                json!("********")
                            }
                        } else if matches!(normalized.as_str(), "base_url" | "proxy" | "url")
                            && value.is_string()
                        {
                            json!(safe_url(value.as_str().unwrap()))
                        } else {
                            sanitize(value)
                        };
                        (key, value)
                    })
                    .collect(),
            )
        }
        Value::Array(values) => Value::Array(values.into_iter().map(sanitize).collect()),
        value => value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secrets_and_url_credentials_are_never_echoed() {
        let raw = json!({"api":"secret","nested":{"aws_secret_key":"aws-secret","headers":{"Authorization":"Bearer private"}},"base_url":"https://name:password@example.org/v1?api_key=secret#secret","ref":{"api":{"$secret":"/api:hash"}}});
        let clean = sanitize(raw);
        assert_eq!(clean["api"], "********");
        assert_eq!(clean["base_url"], "https://example.org/v1");
        assert_eq!(clean["ref"]["api"]["$secret"], "/api:hash");
        assert!(!clean.to_string().contains("aws-secret"));
        assert!(!clean.to_string().contains("Bearer private"));
    }
    #[test]
    fn model_discovery_reuses_saved_credentials_and_raw_url() {
        let saved = json!({"engine":"gpt","base_url":"https://example.invalid/v1?api_key=saved-query","api":["saved-key"],"preferences":{"channel_plugin":"example-plugin","plugin_config":{"credential":"saved-secret","region":"old"}}});
        let draft: DiscoveryChannel = serde_json::from_value(
            json!({"engine":"claude","api":"","plugin_config":{"region":"new"}}),
        )
        .unwrap();
        let channel = discovery_channel(saved.clone(), draft);
        assert_eq!(channel["api"], saved["api"]);
        assert_eq!(channel["base_url"], saved["base_url"]);
        assert_eq!(channel["engine"], "claude");
        assert_eq!(channel["plugin_config"]["credential"], "saved-secret");
        assert_eq!(channel["plugin_config"]["region"], "new");
        let draft: DiscoveryChannel = serde_json::from_value(
            json!({"api":"replacement","base_url":"https://other.invalid/v1"}),
        )
        .unwrap();
        let changed = discovery_channel(saved, draft);
        assert_eq!(changed["api"], "replacement");
        assert_eq!(changed["base_url"], "https://other.invalid/v1");
    }
    #[test]
    fn model_discovery_codex_uses_bundled_catalog_ids() {
        let models = codex_catalog_models();
        assert!(!models.is_empty());
        assert!(models
            .iter()
            .all(|model| model["id"].as_str().is_some_and(|id| !id.is_empty())));
    }
    #[test]
    fn source_projection_uses_current_values_without_runtime_snapshot() {
        let config = json!({"providers":[{"provider":"source","base_url":"https://example.invalid/v1","api":"provider-secret","model":["new-model"]}],"api_keys":[{"api":"admin","role":"user","model":["new-model"]}]});
        let snapshot = compile_source(&config, true).unwrap();
        assert_eq!(snapshot.api_keys[0].role, "user");
        assert_eq!(snapshot.api_keys[0].model_rules, vec![json!("new-model")]);
        assert!(snapshot.providers[0].models.contains_key("new-model"));
    }
    #[test]
    fn plugin_patches_preserve_omitted_and_masked_secrets() {
        let old =
            json!({"plugin_config":{"api_key":"private","credential":"other","region":"old"}});
        let patch = json!({"plugin_config":{"api_key":"********","region":"new"}});
        let merged = channel_preview::merge_channel(&old, &patch).unwrap();
        assert_eq!(merged["plugin_config"]["api_key"], "private");
        assert_eq!(merged["plugin_config"]["credential"], "other");
        assert_eq!(merged["plugin_config"]["region"], "new");
        let unknown = json!({"plugin_config":{"not-existing":"***"}});
        assert!(channel_preview::merge_channel(&old, &unknown).is_err());
    }
    #[test]
    fn policy_header_redaction_preserves_metadata_and_masks_structured_values() {
        let value = json!({"headers":[
            {"name":"Authorization","mode":"set","value":"supplier-secret","secret":false},
            {"name":"X-Custom","mode":"set","value":"private-custom","secret":true},
            {"name":"X-Public","mode":"set","value":"public-value","secret":false}
        ]});
        let clean = sanitize(value);
        assert_eq!(clean["headers"][0]["value"], "********");
        assert_eq!(clean["headers"][0]["secret"], false);
        assert_eq!(clean["headers"][1]["value"], "********");
        assert_eq!(clean["headers"][1]["secret"], true);
        assert_eq!(clean["headers"][2]["value"], "public-value");
        assert_eq!(
            sanitize(json!({"name":"Authorization","value":"standalone-secret","secret":false}))
                ["value"],
            "********"
        );
    }
    #[test]
    fn parameter_patch_preserves_valid_source_extensions() {
        let original = json!({"provider":"existing","base_url":"https://example.invalid/v1","api":"saved-key","model":["m"],"tools":false,"image":0,"vendor_extension":{"keep":null},"preferences":{"vendor_api":{"api_key":"private"},"parameter_policy":{"vendor_extra":{"api_key":"nested-secret"},"channel":{"set":{"vendor":{"api_key":"wire-secret"},"obsolete":1}}}}});
        let patch = json!({"preferences":{"parameter_policy":{"vendor_extra":{"api_key":"********"},"channel":{"set":{"vendor":{"api_key":"********"}}}}}});
        let merged = channel_preview::merge_channel(&original, &patch).unwrap();
        assert!(validate_channel(&merged, true).is_ok());
        assert_eq!(merged["tools"], false);
        assert_eq!(merged["image"], 0);
        assert_eq!(merged["vendor_extension"], original["vendor_extension"]);
        assert_eq!(
            merged.pointer("/preferences/parameter_policy/vendor_extra/api_key"),
            Some(&json!("nested-secret"))
        );
        assert_eq!(
            merged.pointer("/preferences/parameter_policy/channel/set/vendor/api_key"),
            Some(&json!("wire-secret"))
        );
        assert!(merged
            .pointer("/preferences/parameter_policy/channel/set/obsolete")
            .is_none());
    }
    #[test]
    fn channel_updates_cannot_save_masked_keys_or_trigger_discovery() {
        assert!(validate_channel(&json!({"api":"********"}), false).is_err());
        assert!(validate_channel(&json!({"api":{"$secret":"opaque"}}), false).is_err());
        assert!(validate_channel(
            &json!({"provider":"example","base_url":"https://example.org","model":["all"]}),
            true
        )
        .is_err());
        assert!(validate_channel(&json!({"provider":"example","base_url":"https://example.org","model":["model-a"],"api":"new-secret"}), true).is_ok());
        assert!(validate_channel(&json!({"provider":"rename"}), false).is_err());
    }

    #[test]
    fn native_mode_and_pricing_are_validated_before_save() {
        let mut channel = json!({"provider":"example","base_url":"https://example.org/v1","model":["example"],"engine":"gpt","preferences":{"protocol_mode":"native","usage_pricing":{"input":0,"cache_read":null}}});
        assert!(validate_channel(&channel, true).is_ok());
        channel["engine"] = json!("claude");
        assert!(validate_channel(&channel, true).is_ok());
        channel["engine"] = json!("gemini");
        assert!(validate_channel(&channel, true).is_ok());
        channel["engine"] = json!("cohere");
        assert!(validate_channel(&channel, true).is_err());
        channel["engine"] = json!("claude");
        channel["preferences"]["protocol_mode"] = json!("auto");
        assert!(validate_channel(&channel, true).is_ok());
        channel["preferences"]["usage_pricing"]["input"] = json!(-1);
        assert!(validate_channel(&channel, true).is_err());
    }
}
