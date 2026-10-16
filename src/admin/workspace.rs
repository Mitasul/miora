use super::{bounded_body, error, management};
use crate::config::source::{file_revision, RuntimeConfigSource};
use crate::runtime::context::AppState;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{Response, StatusCode, Uri};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Component, Path, PathBuf};

type ApiResult = Result<Value, Response<Body>>;
const MAX_FILE_BYTES: u64 = 1024 * 1024;

fn invalid_path() -> Response<Body> {
    error(
        StatusCode::FORBIDDEN,
        "workspace_path_denied",
        "Only workspace documents and plugin files are accessible",
    )
}

fn relative_path(path: &str) -> Result<PathBuf, Response<Body>> {
    let normalized = path.replace('\\', "/");
    let relative = PathBuf::from(&normalized);
    if relative
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
        || !matches!(
            normalized.split('/').next(),
            Some("docs" | "examples" | "plugins")
        )
    {
        return Err(invalid_path());
    }
    Ok(relative)
}

async fn resolve(path: &str) -> Result<PathBuf, Response<Body>> {
    let relative = relative_path(path)?;
    let base = tokio::fs::canonicalize(".")
        .await
        .map_err(|_| management::unavailable())?;
    let resolved = tokio::fs::canonicalize(base.join(&relative))
        .await
        .map_err(|_| {
            error(
                StatusCode::NOT_FOUND,
                "workspace_file_not_found",
                "Workspace file not found",
            )
        })?;
    let root = base.join(relative.components().next().expect("workspace root"));
    if !resolved.starts_with(root) {
        return Err(invalid_path());
    }
    Ok(resolved)
}

fn text_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|value| value.to_str()),
        Some(
            "md" | "txt"
                | "yaml"
                | "yml"
                | "json"
                | "toml"
                | "py"
                | "rs"
                | "js"
                | "css"
                | "html"
                | "csv"
                | "sh"
        )
    )
}

fn query(uri: &Uri, name: &str) -> String {
    url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes())
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
        .unwrap_or_default()
}

pub(super) async fn read(state: &AppState, uri: &Uri) -> ApiResult {
    let path = query(uri, "path");
    if path.is_empty() {
        let mut entries = vec![
            json!({"path":"config.yaml","name":"源配置","directory":false,"writable":matches!(state.config_publisher.source, RuntimeConfigSource::File(_)),"sensitive":true}),
            json!({"path":"runtime.json","name":"运行状态.json","directory":false,"writable":false}),
            json!({"path":"plugins.json","name":"插件列表.json","directory":false,"writable":false}),
        ];
        for root in ["docs", "examples", "plugins"] {
            if tokio::fs::metadata(root)
                .await
                .is_ok_and(|metadata| metadata.is_dir())
            {
                entries.push(json!({"path":root,"name":root,"directory":true,"writable":false}));
            }
        }
        return Ok(json!({"path":"","entries":entries}));
    }
    if path == "config.yaml" {
        let (content, revision, writable) = match &state.config_publisher.source {
            RuntimeConfigSource::File(source) => {
                let metadata = tokio::fs::metadata(source)
                    .await
                    .map_err(|_| management::unavailable())?;
                if metadata.len() > MAX_FILE_BYTES {
                    return Err(too_large());
                }
                let content = tokio::fs::read_to_string(source)
                    .await
                    .map_err(|_| management::unavailable())?;
                (
                    content,
                    file_revision(&metadata).map_err(|_| management::unavailable())?,
                    true,
                )
            }
            RuntimeConfigSource::Url(_) => {
                let snapshot = state
                    .runtime
                    .snapshot()
                    .await
                    .ok_or_else(management::unavailable)?;
                let config =
                    management::sanitize_for_state(state, snapshot.api_config.as_ref().clone());
                (
                    serde_yaml::to_string(&config).map_err(|_| management::unavailable())?,
                    String::new(),
                    false,
                )
            }
        };
        return Ok(
            json!({"path":path,"content":content,"revision":revision,"writable":writable,"sensitive":true}),
        );
    }
    let virtual_value = match path.as_str() {
        "runtime.json" => Some(state.runtime_observability().await),
        "plugins.json" => Some(state.plugins.list()),
        _ => None,
    };
    if let Some(value) = virtual_value {
        return Ok(
            json!({"path":path,"content":serde_json::to_string_pretty(&management::sanitize_for_state(state, value)).map_err(|_| management::unavailable())?,"writable":false}),
        );
    }
    let resolved = resolve(&path).await?;
    let metadata = tokio::fs::metadata(&resolved)
        .await
        .map_err(|_| management::unavailable())?;
    if metadata.is_dir() {
        let mut directory = tokio::fs::read_dir(&resolved)
            .await
            .map_err(|_| management::unavailable())?;
        let mut entries = Vec::new();
        while let Some(entry) = directory
            .next_entry()
            .await
            .map_err(|_| management::unavailable())?
        {
            let kind = entry
                .file_type()
                .await
                .map_err(|_| management::unavailable())?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if kind.is_symlink()
                || name.starts_with('.')
                || !(kind.is_dir() || text_file(&entry.path()))
            {
                continue;
            }
            entries.push(json!({"path":format!("{path}/{name}"),"name":name,"directory":kind.is_dir(),"writable":false}));
            if entries.len() == 1000 {
                break;
            }
        }
        entries.sort_by(|left, right| {
            right["directory"]
                .as_bool()
                .cmp(&left["directory"].as_bool())
                .then_with(|| left["name"].as_str().cmp(&right["name"].as_str()))
        });
        return Ok(json!({"path":path,"entries":entries}));
    }
    if !text_file(&resolved) {
        return Err(invalid_path());
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Err(too_large());
    }
    let content = tokio::fs::read_to_string(resolved).await.map_err(|_| {
        error(
            StatusCode::BAD_REQUEST,
            "workspace_not_text",
            "File is not UTF-8 text",
        )
    })?;
    Ok(json!({"path":path,"content":content,"writable":false}))
}

fn too_large() -> Response<Body> {
    error(
        StatusCode::PAYLOAD_TOO_LARGE,
        "workspace_file_too_large",
        "Workspace text files are limited to 1 MiB",
    )
}

#[derive(Deserialize)]
struct Save {
    path: String,
    content: String,
    revision: String,
}

pub(super) async fn write(state: &AppState, request: Request) -> ApiResult {
    let raw = bounded_body(request.into_body(), 6 * MAX_FILE_BYTES as usize + 4096).await?;
    let input: Save = serde_json::from_slice(&raw).map_err(|_| {
        error(
            StatusCode::BAD_REQUEST,
            "invalid_workspace_write",
            "Expected path, content and revision",
        )
    })?;
    if input.path != "config.yaml" {
        return Err(invalid_path());
    }
    if input.content.len() as u64 > MAX_FILE_BYTES {
        return Err(too_large());
    }
    let _guard = state.admin.config_lock().try_lock().map_err(|_| {
        error(
            StatusCode::CONFLICT,
            "configuration_busy",
            "Another configuration write is in progress",
        )
    })?;
    state
        .config_publisher
        .replace_source_revision(input.content.as_bytes(), &input.revision)
        .await
        .map_err(management::configuration_error)?;
    state.runtime.refresh().await.map_err(|_| {
        error(
            StatusCode::SERVICE_UNAVAILABLE,
            "runtime_refresh_failed",
            "Configuration saved but runtime refresh failed",
        )
    })?;
    let revision = management::source_revision(state).await?;
    Ok(json!({"ok":true,"revision":revision,"persistence":"source_file"}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_features_paths_do_not_accept_traversal_or_other_roots() {
        for path in [
            "../api.yaml",
            "docs/../api.yaml",
            "/docs/a.md",
            "C:/docs/a.md",
            ".env",
            "target/log.txt",
            "plugins/../../secrets.json",
        ] {
            assert!(relative_path(path).is_err(), "{path}");
        }
        assert!(relative_path("docs/architecture.md").is_ok());
        assert!(relative_path("plugins\\sample.py").is_ok());
        assert!(!text_file(Path::new("database.db")));
    }
}
