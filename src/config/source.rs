use crate::config::discovery::compile_snapshot_with_discovery;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

// The watcher, browser administration and legacy control API must serialize
// read/validate/publish transactions. Multi-process writers are not supported.
static CONFIG_WRITE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(crate) fn file_revision(metadata: &std::fs::Metadata) -> Result<String, String> {
    let modified = metadata
        .modified()
        .map_err(|error| error.to_string())?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| error.to_string())?;
    Ok(format!("{}-{}", modified.as_nanos(), metadata.len()))
}

#[derive(Clone, Debug)]
pub enum RuntimeConfigSource {
    File(PathBuf),
    Url(String),
}

impl RuntimeConfigSource {
    pub fn discover() -> Result<Self, String> {
        let configured_path =
            std::env::var("UNI_API_CONFIG_PATH").unwrap_or_else(|_| "api.yaml".to_owned());
        let path = PathBuf::from(configured_path);
        if path.is_file() {
            return Ok(Self::File(path));
        }
        let configured = std::env::var("CONFIG_URL").unwrap_or_default();
        let trimmed = configured.trim();
        if trimmed.is_empty() {
            return Err(format!(
                "uni-api configuration is unavailable: {} does not exist and CONFIG_URL is unset",
                path.display()
            ));
        }
        if let Some(local) = trimmed.strip_prefix("file://") {
            return Ok(Self::File(PathBuf::from(local)));
        }
        if Path::new(trimmed).is_file() {
            return Ok(Self::File(PathBuf::from(trimmed)));
        }
        if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            return Ok(Self::Url(trimmed.to_owned()));
        }
        Err("CONFIG_URL must be an existing path, file:// URL, or HTTP(S) URL".into())
    }

    pub fn poll_interval(&self) -> std::time::Duration {
        match self {
            Self::File(_) => std::time::Duration::from_secs(2),
            Self::Url(_) => std::time::Duration::from_secs(30),
        }
    }

    pub async fn read(&self, client: &reqwest::Client) -> Result<Vec<u8>, String> {
        match self {
            Self::File(path) => tokio::fs::read(path)
                .await
                .map_err(|error| format!("read {}: {error}", path.display())),
            Self::Url(url) => {
                let response = client
                    .get(url)
                    .send()
                    .await
                    .map_err(|error| format!("fetch runtime configuration: {error}"))?;
                if !response.status().is_success() {
                    return Err(format!(
                        "fetch runtime configuration returned HTTP {}",
                        response.status().as_u16()
                    ));
                }
                response
                    .bytes()
                    .await
                    .map(|bytes| bytes.to_vec())
                    .map_err(|error| format!("read runtime configuration response: {error}"))
            }
        }
    }
}

#[derive(Clone)]
pub struct RuntimeConfigPublisher {
    pub(crate) source: RuntimeConfigSource,
    pub(crate) client: reqwest::Client,
    pub(crate) snapshot_path: Arc<PathBuf>,
    pub(crate) database_disabled: bool,
    pub(crate) discovery_cache: Arc<tokio::sync::Mutex<HashMap<String, Vec<String>>>>,
}

impl RuntimeConfigPublisher {
    pub fn discover(database_disabled: bool) -> Result<Self, String> {
        let source = RuntimeConfigSource::discover()?;
        let client = reqwest::Client::builder()
            .http1_only()
            .build()
            .map_err(|error| format!("build configuration client: {error}"))?;
        let snapshot_path = std::env::var("RUST_RESPONSES_CONFIG_SNAPSHOT_PATH")
            .unwrap_or_else(|_| "/tmp/uni-api-rust-responses-config-v1.json".into());
        Ok(Self {
            source,
            client,
            snapshot_path: Arc::new(PathBuf::from(snapshot_path)),
            database_disabled,
            discovery_cache: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        })
    }

    pub async fn publish(&self) -> Result<(), String> {
        let _guard = CONFIG_WRITE_LOCK.lock().await;
        self.publish_locked().await
    }

    async fn publish_locked(&self) -> Result<(), String> {
        let raw = self.source.read(&self.client).await?;
        let snapshot = compile_snapshot_with_discovery(
            &raw,
            self.database_disabled,
            &self.client,
            &self.discovery_cache,
        )
        .await?;
        atomic_write(self.snapshot_path.as_ref(), &snapshot).await
    }

    pub async fn apply_patch(&self, patch: &Value) -> Result<(), String> {
        self.apply_patch_inner(patch, None).await
    }

    pub async fn apply_patch_revision(
        &self,
        patch: &Value,
        expected_revision: &str,
    ) -> Result<(), String> {
        self.apply_patch_inner(patch, Some(expected_revision)).await
    }

    pub(crate) async fn replace_source_revision(
        &self,
        raw: &[u8],
        expected: &str,
    ) -> Result<(), String> {
        let _guard = CONFIG_WRITE_LOCK.lock().await;
        let RuntimeConfigSource::File(path) = &self.source else {
            return Err("remote configuration is read-only".into());
        };
        let metadata = tokio::fs::metadata(path)
            .await
            .map_err(|error| error.to_string())?;
        if file_revision(&metadata)? != expected {
            return Err("configuration revision conflict".into());
        }
        let original = tokio::fs::read(path)
            .await
            .map_err(|error| error.to_string())?;
        self.commit_bytes_locked(path, raw, &original).await
    }

    async fn apply_patch_inner(
        &self,
        patch: &Value,
        expected_revision: Option<&str>,
    ) -> Result<(), String> {
        let _guard = CONFIG_WRITE_LOCK.lock().await;
        let RuntimeConfigSource::File(path) = &self.source else {
            return Err("runtime configuration is read-only when CONFIG_URL is remote".into());
        };
        if let Some(expected) = expected_revision {
            let metadata = tokio::fs::metadata(path)
                .await
                .map_err(|error| error.to_string())?;
            if expected != file_revision(&metadata)? {
                return Err("configuration revision conflict".into());
            }
        }
        let raw = tokio::fs::read(path)
            .await
            .map_err(|error| format!("read {} for update: {error}", path.display()))?;
        let yaml: serde_yaml::Value = serde_yaml::from_slice(&raw)
            .map_err(|error| format!("decode configuration for update: {error}"))?;
        let mut config = serde_json::to_value(yaml)
            .map_err(|error| format!("convert configuration for update: {error}"))?;
        let root = config
            .as_object_mut()
            .ok_or_else(|| "uni-api configuration must be a mapping".to_owned())?;
        let patch = patch
            .as_object()
            .ok_or_else(|| "configuration patch must be an object".to_owned())?;
        for (key, value) in patch {
            if matches!(
                key.as_str(),
                "providers" | "api_keys" | "preferences" | "video" | "virtual_models"
            ) {
                root.insert(key.clone(), value.clone());
            }
        }
        self.commit_config_locked(path, &config, &raw).await
    }

    // Validate discovery before committing either file. If publishing fails,
    // restore the source while the watcher is excluded by CONFIG_WRITE_LOCK.
    async fn commit_config_locked(
        &self,
        path: &Path,
        config: &Value,
        original: &[u8],
    ) -> Result<(), String> {
        let encoded = serde_yaml::to_string(config)
            .map_err(|error| format!("encode configuration: {error}"))?;
        self.commit_bytes_locked(path, encoded.as_bytes(), original)
            .await
    }

    async fn commit_bytes_locked(
        &self,
        path: &Path,
        encoded: &[u8],
        original: &[u8],
    ) -> Result<(), String> {
        let snapshot = compile_snapshot_with_discovery(
            encoded,
            self.database_disabled,
            &self.client,
            &self.discovery_cache,
        )
        .await?;
        let current = tokio::fs::read(path)
            .await
            .map_err(|error| format!("verify configuration: {error}"))?;
        if current != original {
            return Err("configuration revision conflict".into());
        }
        atomic_write(path, encoded).await?;
        if let Err(error) = atomic_write(self.snapshot_path.as_ref(), &snapshot).await {
            atomic_write(path, original)
                .await
                .map_err(|rollback| format!("{error}; source rollback failed: {rollback}"))?;
            return Err(error);
        }
        Ok(())
    }

    pub async fn add_credits(&self, paid_key: &str, amount: f64) -> Result<f64, String> {
        let _guard = CONFIG_WRITE_LOCK.lock().await;
        if !amount.is_finite() || amount <= 0.0 {
            return Err("amount must be positive".into());
        }
        let RuntimeConfigSource::File(path) = &self.source else {
            return Err("runtime configuration is read-only when CONFIG_URL is remote".into());
        };
        let raw = tokio::fs::read(path)
            .await
            .map_err(|error| format!("read {} for credits update: {error}", path.display()))?;
        let yaml: serde_yaml::Value = serde_yaml::from_slice(&raw)
            .map_err(|error| format!("decode configuration for credits update: {error}"))?;
        let mut config = serde_json::to_value(yaml)
            .map_err(|error| format!("convert configuration for credits update: {error}"))?;
        let keys = config
            .get_mut("api_keys")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| "configuration contains no API keys".to_owned())?;
        let item = keys
            .iter_mut()
            .find(|item| item.get("api").and_then(Value::as_str) == Some(paid_key))
            .ok_or_else(|| "Paid API key not found".to_owned())?;
        let preferences = item
            .as_object_mut()
            .expect("API key configuration object")
            .entry("preferences")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or_else(|| "API key preferences must be an object".to_owned())?;
        let current = preferences
            .get("credits")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let updated = current + amount;
        preferences.insert("credits".into(), json!(updated));
        self.commit_config_locked(path, &config, &raw).await?;
        Ok(updated)
    }

    pub fn start_watcher(&self) {
        let publisher = self.clone();
        tokio::spawn(async move {
            let mut last_snapshot = Value::Null;
            loop {
                let guard = CONFIG_WRITE_LOCK.lock().await;
                let snapshot = match publisher.source.read(&publisher.client).await {
                    Ok(raw) => {
                        compile_snapshot_with_discovery(
                            &raw,
                            publisher.database_disabled,
                            &publisher.client,
                            &publisher.discovery_cache,
                        )
                        .await
                    }
                    Err(error) => Err(error),
                };
                match snapshot {
                    Ok(snapshot) => {
                        let mut content: Value =
                            serde_json::from_slice(&snapshot).expect("compiled snapshot JSON");
                        let object = content.as_object_mut().expect("compiled snapshot object");
                        object.remove("revision");
                        object.remove("generated_unix_ms");
                        if content != last_snapshot {
                            match atomic_write(publisher.snapshot_path.as_ref(), &snapshot).await {
                                Ok(()) => last_snapshot = content,
                                Err(error) => crate::observability::backend_logs::event(json!({
                                    "event_type": "rust_runtime_config_publish_error",
                                    "error": error,
                                })),
                            }
                        }
                    }
                    Err(error) => crate::observability::backend_logs::event(json!({
                        "event_type": "rust_runtime_config_reload_error",
                        "error": error,
                    })),
                }
                drop(guard);
                tokio::time::sleep(publisher.source.poll_interval()).await;
            }
        });
    }
}

pub(crate) async fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|error| format!("create snapshot directory {}: {error}", parent.display()))?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("snapshot"),
        std::process::id(),
    ));
    tokio::fs::write(&temporary, bytes)
        .await
        .map_err(|error| format!("write {}: {error}", temporary.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600))
            .await
            .map_err(|error| format!("chmod {}: {error}", temporary.display()))?;
    }
    tokio::fs::rename(&temporary, path)
        .await
        .map_err(|error| format!("publish {}: {error}", path.display()))
}

#[cfg(test)]
mod tests;
