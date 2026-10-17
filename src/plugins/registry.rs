use super::interceptors::{validate_chain, HeaderInterceptor};
use super::manifest::Manifest;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock, Weak};
use url::Url;

// Deterministic test interleaving after persistence and before the Store swap.
// No hook or additional state exists in release builds or the pin hot path.
#[cfg(test)]
std::thread_local! {
    static BEFORE_PUBLICATION: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
pub(super) fn before_next_publication(hook: impl FnOnce() + 'static) {
    BEFORE_PUBLICATION.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some(Box::new(hook));
    });
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServiceAuth {
    #[serde(default = "service_none", rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub token: String,
}
fn service_none() -> String {
    "none".into()
}
impl Default for ServiceAuth {
    fn default() -> Self {
        Self {
            kind: service_none(),
            token: String::new(),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Registration {
    pub manifest: Manifest,
    pub endpoint: String,
    #[serde(default)]
    pub config: Map<String, Value>,
    #[serde(default)]
    pub service_auth: ServiceAuth,
    pub expected_revision: u64,
    #[serde(skip)]
    pub managed: Option<super::managed::Package>,
    #[serde(skip)]
    pub runtime: Option<Arc<super::managed::Process>>,
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Version {
    pub revision: u64,
    pub manifest: Manifest,
    pub endpoint: String,
    pub config: Map<String, Value>,
    pub service_auth: ServiceAuth,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed: Option<super::managed::Package>,
    #[serde(skip)]
    pub runtime: Option<Arc<super::managed::Process>>,
}
#[derive(Clone)]
struct Entry {
    enabled: bool,
    revision: u64,
    active: u64,
    versions: BTreeMap<u64, Arc<Version>>,
}
#[derive(Clone)]
struct Store {
    generation: u64,
    entries: BTreeMap<String, Entry>,
    interceptor_revision: u64,
    interceptors: Arc<Vec<HeaderInterceptor>>,
    // The only authoritative execution projection. Rebuilt on successful writes,
    // never while pinning a request.
    published: Arc<Snapshot>,
    // Liveness bookkeeping for cold management views, not another version cache.
    retired: Vec<Weak<Snapshot>>,
}
impl Default for Store {
    fn default() -> Self {
        let interceptors = Arc::new(Vec::new());
        Self {
            generation: 0,
            entries: BTreeMap::new(),
            interceptor_revision: 0,
            published: Arc::new(Snapshot {
                versions: BTreeMap::new(),
                interceptors: interceptors.clone(),
            }),
            interceptors,
            retired: Vec::new(),
        }
    }
}
impl Store {
    fn rebuild_snapshot(&mut self) {
        self.published = Arc::new(Snapshot {
            versions: self
                .entries
                .iter()
                .filter(|(_, entry)| entry.enabled)
                .map(|(id, entry)| (id.clone(), entry.versions[&entry.active].clone()))
                .collect(),
            interceptors: self.interceptors.clone(),
        });
    }
    fn lease_counts(&self) -> BTreeMap<String, BTreeMap<u64, usize>> {
        let mut counts = BTreeMap::<String, BTreeMap<u64, usize>>::new();
        let mut add = |snapshot: &Snapshot, references: usize| {
            if references == 0 {
                return;
            }
            for (id, version) in &snapshot.versions {
                *counts
                    .entry(id.clone())
                    .or_default()
                    .entry(version.revision)
                    .or_default() += references;
            }
        };
        // Counts are live snapshot Arc holders, not a count of unique requests:
        // a request may hold both a pin and response/capture leases. Exclude the
        // registry's own published Arc and our temporary Weak upgrades.
        add(
            &self.published,
            Arc::strong_count(&self.published).saturating_sub(1),
        );
        for retired in &self.retired {
            if let Some(snapshot) = retired.upgrade() {
                add(&snapshot, Arc::strong_count(&snapshot).saturating_sub(1));
            }
        }
        counts
    }
}
#[derive(Serialize, Deserialize)]
struct SavedEntry {
    enabled: bool,
    revision: u64,
    active: u64,
    versions: Vec<Version>,
}
#[derive(Serialize, Deserialize)]
struct SavedStore {
    format: u32,
    generation: u64,
    entries: BTreeMap<String, SavedEntry>,
    interceptor_revision: u64,
    interceptors: Vec<HeaderInterceptor>,
}
#[derive(Clone)]
pub(crate) struct TrustPolicy {
    pub allowed_origins: Vec<String>,
    pub allow_loopback_http: bool,
}
impl TrustPolicy {
    pub fn validate(&self, endpoint: &str) -> Result<Url, String> {
        let url = Url::parse(endpoint).map_err(|_| "invalid plugin endpoint".to_owned())?;
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.host_str().is_none()
        {
            return Err("plugin endpoint cannot contain credentials, query or fragment".into());
        }
        let loopback = matches!(url.host(), Some(url::Host::Ipv4(ip)) if ip.is_loopback())
            || matches!(url.host(), Some(url::Host::Ipv6(ip)) if ip.is_loopback());
        let origin = url.origin().ascii_serialization();
        if url.scheme() == "http" && loopback && self.allow_loopback_http {
            return Ok(url);
        }
        if url.scheme() != "https" || !self.allowed_origins.contains(&origin) {
            return Err("plugin endpoint is not trusted by startup policy".into());
        }
        Ok(url)
    }
}
#[derive(Clone)]
pub(crate) struct Snapshot {
    pub versions: BTreeMap<String, Arc<Version>>,
    pub interceptors: Arc<Vec<HeaderInterceptor>>,
}
struct RegistryConfig {
    path: Option<PathBuf>,
    trust: TrustPolicy,
}
pub(crate) struct PluginRegistry {
    store: Arc<RwLock<Store>>,
    config: Arc<RegistryConfig>,
    pub client: reqwest::Client,
    execution_clients: Arc<
        std::sync::Mutex<std::collections::HashMap<Option<std::time::Duration>, reqwest::Client>>,
    >,
    pinned: Option<Arc<Snapshot>>,
    jobs: Arc<std::sync::Mutex<BTreeMap<String, InstallJob>>>,
}

struct InstallJob {
    updating: bool,
    error: Option<String>,
    logs: super::managed::Logs,
}

struct JobGuard {
    jobs: Arc<std::sync::Mutex<BTreeMap<String, InstallJob>>>,
    id: String,
}
impl Drop for JobGuard {
    fn drop(&mut self) {
        if let Some(job) = self.jobs.lock().unwrap().get_mut(&self.id) {
            if job.updating {
                job.updating = false;
                job.error = Some("installation cancelled".into());
            }
        }
    }
}
impl PluginRegistry {
    pub fn from_env() -> Result<Self, String> {
        let trust = TrustPolicy {
            allowed_origins: std::env::var("MIORA_PLUGIN_ALLOWED_ORIGINS")
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect(),
            allow_loopback_http: std::env::var("MIORA_PLUGIN_ALLOW_LOOPBACK_HTTP")
                .ok()
                .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true")),
        };
        let path = std::env::var("MIORA_PLUGIN_STORE_DIR")
            .unwrap_or_else(|_| "data/channel-plugins".into());
        Self::open(Some(PathBuf::from(path)), trust)
    }
    pub fn open(path: Option<PathBuf>, trust: TrustPolicy) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|_| "build plugin HTTP client".to_owned())?;
        let registry = Self {
            store: Arc::new(RwLock::new(Store::default())),
            config: Arc::new(RegistryConfig { path, trust }),
            client,
            execution_clients: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            pinned: None,
            jobs: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
        };
        if let Some(path) = &registry.config.path {
            if path.exists() {
                let mut files = std::fs::read_dir(path)
                    .map_err(|_| "read plugin store".to_owned())?
                    .filter_map(Result::ok)
                    .map(|e| e.path())
                    .filter(|p| {
                        p.file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.starts_with("revision-") && n.ends_with(".json"))
                    })
                    .collect::<Vec<_>>();
                files.sort();
                if let Some(file) = files.last() {
                    let bytes =
                        std::fs::read(file).map_err(|_| "read plugin store revision".to_owned())?;
                    if bytes.len() > 64 * 1024 * 1024 {
                        return Err("plugin store exceeds size limit".into());
                    }
                    let saved: SavedStore = serde_json::from_slice(&bytes)
                        .map_err(|_| "invalid plugin store revision".to_owned())?;
                    if saved.format != 1 {
                        return Err("unsupported plugin store format".into());
                    }
                    let mut store = Store {
                        generation: saved.generation,
                        interceptor_revision: saved.interceptor_revision,
                        ..Default::default()
                    };
                    let mut chain = saved.interceptors;
                    validate_chain(&mut chain)?;
                    store.interceptors = Arc::new(chain);
                    for (id, entry) in saved.entries {
                        let mut versions = BTreeMap::new();
                        for version in entry.versions {
                            version.manifest.validate()?;
                            if version.manifest.id != id
                                || version.revision > entry.revision
                                || versions.contains_key(&version.revision)
                            {
                                return Err("invalid persisted plugin history".into());
                            }
                            // Fail closed when startup trust is tightened; never silently bypass it.
                            registry.validate_version(&version)?;
                            versions.insert(version.revision, Arc::new(version));
                        }
                        if !versions.contains_key(&entry.active) {
                            return Err("missing active plugin revision".into());
                        }
                        store.entries.insert(
                            id,
                            Entry {
                                enabled: entry.enabled,
                                revision: entry.revision,
                                active: entry.active,
                                versions,
                            },
                        );
                    }
                    store.rebuild_snapshot();
                    *registry.store.write().unwrap() = store;
                }
            }
        }
        Ok(registry)
    }
    fn packages_root(&self) -> Result<PathBuf, String> {
        self.config
            .path
            .as_ref()
            .map(|path| path.join("packages"))
            .ok_or_else(|| "managed plugins require a persistent plugin store".into())
    }

    pub(super) async fn stage_managed(
        &self,
        bytes: &[u8],
        extract: bool,
    ) -> Result<(super::managed::Staging, Value), String> {
        let staging = super::managed::Staging::new(&self.packages_root()?)?;
        let metadata = super::managed::unpack(bytes, &staging, extract).await?;
        Ok((staging, metadata))
    }

    pub(super) async fn install_managed(
        &self,
        mut staging: super::managed::Staging,
        metadata: Value,
        expected: u64,
    ) -> Result<Value, String> {
        let manifest: Manifest = serde_json::from_value(metadata["manifest"].clone())
            .map_err(|error| format!("read managed manifest: {error}"))?;
        let id = manifest.id.clone();
        {
            let store = self.store.read().unwrap();
            let old = store.entries.get(&id);
            if old.map_or(0, |entry| entry.revision) != expected {
                return Err("revision_conflict".into());
            }
        }
        let logs = super::managed::Logs::default();
        {
            let mut jobs = self.jobs.lock().unwrap();
            if jobs.get(&id).is_some_and(|job| job.updating) {
                return Err("plugin update already in progress".into());
            }
            if jobs.len() >= 128 && !jobs.contains_key(&id) {
                let finished = jobs
                    .iter()
                    .find(|(_, job)| !job.updating)
                    .map(|(id, _)| id.clone())
                    .ok_or("too many concurrent plugin installations")?;
                jobs.remove(&finished);
            }
            jobs.insert(
                id.clone(),
                InstallJob {
                    updating: true,
                    error: None,
                    logs: logs.clone(),
                },
            );
        }
        let _job = JobGuard {
            jobs: self.jobs.clone(),
            id: id.clone(),
        };
        let result = async {
            super::managed::prepare(&staging, &logs).await?;
            use ring::rand::SecureRandom;
            let mut bytes = [0u8; 32];
            ring::rand::SystemRandom::new()
                .fill(&mut bytes)
                .map_err(|_| "generate plugin service credential")?;
            let token = hex::encode(bytes);
            let package = super::managed::Package {
                directory: staging.name.clone(),
                requirements: serde_json::from_value(metadata["requirements"].clone())
                    .map_err(|_| "invalid plugin dependencies")?,
            };
            let (runtime, endpoint) =
                super::managed::start(&self.packages_root()?, &package, &id, &token, logs.clone())
                    .await?;
            let registration = Registration {
                manifest,
                endpoint,
                config: Map::new(),
                service_auth: ServiceAuth {
                    kind: "bearer".into(),
                    token,
                },
                expected_revision: expected,
                managed: Some(package),
                runtime: Some(runtime),
            };
            self.register_checked(registration).await
        }
        .await;
        {
            let mut jobs = self.jobs.lock().unwrap();
            let job = jobs.get_mut(&id).unwrap();
            job.updating = false;
            job.error = result.as_ref().err().cloned();
        }
        if result.is_ok() {
            staging.retain();
        }
        result
    }

    async fn start_version(
        &self,
        version: &mut Version,
        logs: super::managed::Logs,
    ) -> Result<(), String> {
        let package = version.managed.as_ref().ok_or("not a managed plugin")?;
        let (runtime, endpoint) = super::managed::start(
            &self.packages_root()?,
            package,
            &version.manifest.id,
            &version.service_auth.token,
            logs,
        )
        .await?;
        version.endpoint = endpoint;
        version.runtime = Some(runtime);
        Ok(())
    }

    pub async fn restore_managed(&self) {
        let versions: Vec<_> = self
            .snapshot()
            .versions
            .values()
            .filter(|version| version.managed.is_some())
            .cloned()
            .collect();
        for original in versions {
            let mut version = (*original).clone();
            let logs = super::managed::Logs::default();
            let result = async {
                self.start_version(&mut version, logs.clone()).await?;
                self.health(&version).await
            }
            .await;
            if let Err(error) = result {
                self.jobs.lock().unwrap().insert(
                    version.manifest.id.clone(),
                    InstallJob {
                        updating: false,
                        error: Some(error),
                        logs,
                    },
                );
                continue;
            }
            let mut store = self.store.write().unwrap();
            if let Some(entry) = store.entries.get_mut(&version.manifest.id) {
                entry.versions.insert(version.revision, Arc::new(version));
            }
            store.rebuild_snapshot();
        }
    }

    pub fn logs(&self, id: &str) -> Result<Value, String> {
        let store = self.store.read().unwrap();
        let version = store
            .entries
            .get(id)
            .map(|entry| &entry.versions[&entry.active]);
        let jobs = self.jobs.lock().unwrap();
        if version.is_some_and(|version| version.managed.is_none()) && !jobs.contains_key(id) {
            return Ok(json!({"plugin_id":id,"chunks":[],"external":true}));
        }
        let logs = jobs
            .get(id)
            .map(|job| job.logs.clone())
            .or_else(|| {
                version.and_then(|version| {
                    version.runtime.as_ref().map(|runtime| runtime.logs.clone())
                })
            })
            .ok_or("no managed plugin logs available")?;
        let mut chunks: Vec<String> = logs.lock().unwrap().iter().cloned().collect();
        if let Some(version) = version {
            let secrets = std::iter::once(version.service_auth.token.as_str()).chain(
                version
                    .manifest
                    .config_schema
                    .iter()
                    .filter(|field| field.secret)
                    .filter_map(|field| version.config.get(&field.key).and_then(Value::as_str)),
            );
            for secret in secrets.filter(|secret| !secret.is_empty()) {
                for chunk in &mut chunks {
                    *chunk = chunk.replace(secret, "***");
                }
            }
        }
        Ok(
            json!({"plugin_id":id,"chunks":chunks,"bounded_bytes":409600,"last_error":jobs.get(id).and_then(|job| job.error.clone())}),
        )
    }

    pub fn validate_instances(
        &self,
        manifest: &Manifest,
        supplied: Option<&Map<String, Value>>,
        providers: &[Arc<crate::config::snapshot::Provider>],
    ) -> Result<(), String> {
        let current = self.store.read().unwrap();
        let mut defaults = current
            .entries
            .get(&manifest.id)
            .map(|entry| entry.versions[&entry.active].config.clone())
            .unwrap_or_default();
        defaults.retain(|key, _| {
            manifest.config_schema.iter().any(|field| field.key == *key)
                || manifest.config_aliases.values().any(|source| source == key)
        });
        if let Some(supplied) = supplied {
            for (key, value) in supplied {
                if value != "***" {
                    defaults.insert(key.clone(), value.clone());
                }
            }
        }
        Self::check_instances(manifest, &defaults, providers, &current.published)
    }

    fn check_instances(
        manifest: &Manifest,
        defaults: &Map<String, Value>,
        providers: &[Arc<crate::config::snapshot::Provider>],
        snapshot: &Snapshot,
    ) -> Result<(), String> {
        for provider in providers {
            let channel_id = super::runtime::channel_plugin_id(provider);
            let mut config = defaults.clone();
            if channel_id == Some(manifest.id.as_str()) {
                if let Some(overrides) = provider
                    .preferences
                    .get("plugin_config")
                    .and_then(Value::as_object)
                {
                    config.extend(overrides.clone());
                }
                if !manifest.auth.config_key.is_empty() {
                    if let Some(key) = provider
                        .api_keys
                        .first()
                        .filter(|key| key.as_str() != "__channel_plugin__")
                    {
                        config.insert(manifest.auth.config_key.clone(), json!(key));
                    }
                }
            } else if manifest.capabilities.balance_query
                || provider
                    .preferences
                    .get("balance_plugin")
                    .and_then(Value::as_str)
                    == Some(manifest.id.as_str())
            {
                let source = channel_id
                    .and_then(|id| snapshot.versions.get(id))
                    .map(AsRef::as_ref);
                let uses_balance = match provider
                    .preferences
                    .get("balance_plugin")
                    .and_then(Value::as_str)
                {
                    Some(id) => id == manifest.id,
                    None if source
                        .is_some_and(|version| version.manifest.capabilities.balance_query) =>
                    {
                        false
                    }
                    None => {
                        let source_config = Self::balance_source_config(provider, source)?;
                        Self::matches_balance_target(manifest, provider, source, &source_config)
                    }
                };
                if !uses_balance {
                    continue;
                }
                if !manifest.capabilities.balance_query {
                    return Err(format!("channel {} requires balance_query", provider.name));
                }
                let (overrides, key) = Self::balance_configuration(
                    manifest,
                    provider,
                    provider
                        .api_keys
                        .first()
                        .map(String::as_str)
                        .unwrap_or_default(),
                    source,
                )?;
                config.extend(overrides);
                if !manifest.auth.config_key.is_empty()
                    && !key.is_empty()
                    && key != "__channel_plugin__"
                {
                    config.insert(manifest.auth.config_key.clone(), json!(key));
                }
            } else {
                continue;
            }
            manifest
                .config(&config, true)
                .map_err(|error| format!("channel {} is incompatible: {error}", provider.name))?;
        }
        Ok(())
    }

    pub fn validate_rollback_instances(
        &self,
        id: &str,
        revision: u64,
        providers: &[Arc<crate::config::snapshot::Provider>],
    ) -> Result<(), String> {
        let store = self.store.read().unwrap();
        let version = store
            .entries
            .get(id)
            .ok_or("plugin_not_found")?
            .versions
            .get(&revision)
            .ok_or("revision_not_found")?;
        Self::check_instances(
            &version.manifest,
            &version.config,
            providers,
            &store.published,
        )
    }

    pub fn execution_client(
        &self,
        connect_timeout: Option<std::time::Duration>,
    ) -> Result<reqwest::Client, String> {
        let mut clients = self.execution_clients.lock().unwrap();
        if let Some(client) = clients.get(&connect_timeout) {
            return Ok(client.clone());
        }
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy();
        if let Some(timeout) = connect_timeout {
            builder = builder.connect_timeout(timeout);
        }
        let client = builder
            .build()
            .map_err(|_| "build plugin execution client".to_owned())?;
        // Configured timeout diversity must not produce an unbounded pool.
        if clients.len() >= 64 {
            clients.clear();
        }
        clients.insert(connect_timeout, client.clone());
        Ok(client)
    }
    pub fn pin(&self) -> Self {
        Self {
            store: self.store.clone(),
            config: self.config.clone(),
            client: self.client.clone(),
            execution_clients: self.execution_clients.clone(),
            pinned: Some(self.snapshot()),
            jobs: self.jobs.clone(),
        }
    }
    pub fn snapshot(&self) -> Arc<Snapshot> {
        if let Some(snapshot) = &self.pinned {
            return snapshot.clone();
        }
        self.store.read().unwrap().published.clone()
    }
    fn validate_version(&self, version: &Version) -> Result<Map<String, Value>, String> {
        version.manifest.validate()?;
        let url = if let Some(package) = &version.managed {
            super::managed::package_path(&self.packages_root()?, package)?;
            let url = Url::parse(&version.endpoint).map_err(|_| "invalid managed endpoint")?;
            if url.scheme() != "http" || url.host_str() != Some("127.0.0.1") {
                return Err("managed plugins must use a local runtime endpoint".into());
            }
            url
        } else {
            self.config.trust.validate(&version.endpoint)?
        };
        let config = version.manifest.config(&version.config, false)?;
        match version.service_auth.kind.as_str() {
            "bearer"
                if !version.service_auth.token.is_empty()
                    && version.service_auth.token != "***"
                    && version.service_auth.token.len() <= 4096 =>
            {
                axum::http::HeaderValue::from_str(&format!(
                    "Bearer {}",
                    version.service_auth.token
                ))
                .map_err(|_| "invalid plugin service credential".to_owned())?;
            }
            "" | "none" if url.scheme() == "http" && self.config.trust.allow_loopback_http => {
                if !version.service_auth.token.is_empty() {
                    return Err("unexpected service credential".into());
                }
            }
            _ => {
                return Err(
                    "trusted HTTPS plugin services require independent bearer service_auth".into(),
                )
            }
        }
        Ok(config)
    }
    fn persist(&self, store: &Store) -> Result<(), String> {
        let Some(path) = &self.config.path else {
            return Ok(());
        };
        std::fs::create_dir_all(path).map_err(|_| "create plugin store directory".to_owned())?;
        let saved = SavedStore {
            format: 1,
            generation: store.generation,
            entries: store
                .entries
                .iter()
                .map(|(id, e)| {
                    (
                        id.clone(),
                        SavedEntry {
                            enabled: e.enabled,
                            revision: e.revision,
                            active: e.active,
                            versions: e.versions.values().map(|v| (**v).clone()).collect(),
                        },
                    )
                })
                .collect(),
            interceptor_revision: store.interceptor_revision,
            interceptors: (*store.interceptors).clone(),
        };
        let bytes = serde_json::to_vec(&saved).map_err(|_| "encode plugin store".to_owned())?;
        if bytes.len() > 64 * 1024 * 1024 {
            return Err("plugin store exceeds size limit".into());
        }
        // Append-only immutable generations work on both Windows and Unix. A failed
        // write never replaces the committed file or in-memory registry.
        let target = path.join(format!("revision-{:020}.json", store.generation));
        if target.exists() {
            return Err("plugin store generation conflict (single writer required)".into());
        }
        let temporary = path.join(format!(
            "pending-{}-{}.tmp",
            std::process::id(),
            store.generation
        ));
        let result = (|| -> Result<(), std::io::Error> {
            use std::io::Write;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temporary, &target)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result.map_err(|_| "persist plugin revision (previous version retained)".to_owned())
    }
    fn publish(&self, guard: &mut Store, mut next: Store) -> Result<(), String> {
        next.generation = guard
            .generation
            .checked_add(1)
            .ok_or("plugin generation exhausted")?;
        for entry in next.entries.values_mut() {
            for (revision, version) in &mut entry.versions {
                if (!entry.enabled || *revision != entry.active) && version.runtime.is_some() {
                    Arc::make_mut(version).runtime = None;
                }
            }
        }
        next.rebuild_snapshot();
        next.retired.retain(|snapshot| snapshot.strong_count() > 0);
        next.retired.push(Arc::downgrade(&guard.published));
        self.persist(&next)?;
        #[cfg(test)]
        BEFORE_PUBLICATION.with(|hook| {
            if let Some(hook) = hook.borrow_mut().take() {
                hook();
            }
        });
        *guard = next;
        if let Some(path) = &self.config.path {
            if let Ok(files) = std::fs::read_dir(path) {
                for file in files.flatten() {
                    let name = file.file_name().to_string_lossy().into_owned();
                    if name
                        .strip_prefix("revision-")
                        .and_then(|name| name.strip_suffix(".json"))
                        .and_then(|value| value.parse::<u64>().ok())
                        .is_some_and(|generation| generation < guard.generation)
                    {
                        let _ = std::fs::remove_file(file.path());
                    }
                }
            }
        }
        Ok(())
    }
    fn candidate(&self, guard: &Store, mut request: Registration) -> Result<Version, String> {
        let id = request.manifest.id.clone();
        let old = guard.entries.get(&id);
        if old.map_or(0, |e| e.revision) != request.expected_revision {
            return Err("revision_conflict".into());
        }
        if old.is_none() && guard.entries.len() >= 128 {
            return Err("plugin limit exceeded".into());
        }
        if old.is_some_and(|e| e.versions.len() >= 100) {
            return Err("plugin history limit reached (100 versions)".into());
        }
        if let Some(old) = old {
            let prior = &old.versions[&old.active];
            for (key, value) in &prior.config {
                if request
                    .manifest
                    .config_schema
                    .iter()
                    .any(|field| field.key == *key)
                    || request
                        .manifest
                        .config_aliases
                        .values()
                        .any(|source| source == key)
                {
                    request
                        .config
                        .entry(key.clone())
                        .or_insert_with(|| value.clone());
                }
            }
            for field in request.manifest.config_schema.iter().filter(|f| f.secret) {
                if request.config.get(&field.key).and_then(Value::as_str) == Some("***") {
                    let value = prior
                        .config
                        .get(&field.key)
                        .ok_or("missing previous secret")?
                        .clone();
                    request.config.insert(field.key.clone(), value);
                }
            }
            if request.service_auth.token == "***" {
                request.service_auth.token = prior.service_auth.token.clone();
            }
        }
        let revision = request
            .expected_revision
            .checked_add(1)
            .ok_or("plugin revision exhausted")?;
        let endpoint = request.endpoint.trim_end_matches('/').to_owned();
        let mut version = Version {
            revision,
            manifest: request.manifest,
            endpoint,
            config: request.config,
            service_auth: request.service_auth,
            managed: request.managed,
            runtime: request.runtime,
        };
        version.config = self.validate_version(&version)?;
        Ok(version)
    }
    /// Validate and probe a candidate before publication. No lock is held over
    /// the network operation; publication repeats CAS before committing the result.
    pub async fn register_checked(&self, request: Registration) -> Result<Value, String> {
        let expected = request.expected_revision;
        let mut candidate = {
            let guard = self.store.read().unwrap();
            self.candidate(&guard, request)?
        };
        if candidate.managed.is_some() && candidate.runtime.is_none() {
            self.start_version(&mut candidate, Default::default())
                .await?;
        }
        self.health(&candidate).await?;
        let mut guard = self.store.write().unwrap();
        let old = guard.entries.get(&candidate.manifest.id);
        if old.map_or(0, |entry| entry.revision) != expected {
            return Err("revision_conflict".into());
        }
        if old.is_none() && guard.entries.len() >= 128 {
            return Err("plugin limit exceeded".into());
        }
        self.publish_registration(&mut guard, candidate)
    }
    pub async fn change_checked(
        &self,
        id: &str,
        expected: u64,
        enabled: Option<bool>,
        rollback: Option<u64>,
    ) -> Result<Value, String> {
        if enabled == Some(true) || rollback.is_some() {
            let version = {
                let guard = self.store.read().unwrap();
                let entry = guard.entries.get(id).ok_or("plugin_not_found")?;
                if entry.revision != expected {
                    return Err("revision_conflict".into());
                }
                entry
                    .versions
                    .get(&rollback.unwrap_or(entry.active))
                    .ok_or("revision_not_found")?
                    .clone()
            };
            let mut version = (*version).clone();
            let logs = super::managed::Logs::default();
            let preparation = async {
                if version.managed.is_some()
                    && version
                        .runtime
                        .as_ref()
                        .is_none_or(|runtime| !runtime.running())
                {
                    self.start_version(&mut version, logs.clone()).await?;
                }
                self.health(&version).await
            }
            .await;
            if let Err(error) = preparation {
                if version.managed.is_some() {
                    self.jobs.lock().unwrap().insert(
                        id.into(),
                        InstallJob {
                            updating: false,
                            error: Some(error.clone()),
                            logs,
                        },
                    );
                }
                return Err(error);
            }
            let mut guard = self.store.write().unwrap();
            let entry = guard.entries.get(id).ok_or("plugin_not_found")?;
            if entry.revision != expected {
                return Err("revision_conflict".into());
            }
            let mut next = guard.clone();
            let entry = next.entries.get_mut(id).unwrap();
            entry.active = version.revision;
            entry.versions.insert(version.revision, Arc::new(version));
            if let Some(enabled) = enabled {
                entry.enabled = enabled;
            }
            entry.revision = expected + 1;
            self.publish(&mut guard, next)?;
            self.jobs.lock().unwrap().remove(id);
            return Ok(entry_view(id, &guard.entries[id], &guard.lease_counts()));
        }
        self.change(id, expected, enabled, rollback)
    }
    async fn health(&self, candidate: &Version) -> Result<(), String> {
        use futures_util::StreamExt;
        let mut request = self
            .client
            .get(format!("{}/health", candidate.endpoint))
            .timeout(std::time::Duration::from_secs(10))
            .header("x-miora-plugin-version", &candidate.manifest.version);
        if candidate.service_auth.kind == "bearer" {
            request = request.bearer_auth(&candidate.service_auth.token);
        }
        let response = request.send().await.map_err(|_| {
            "plugin health check transport failure (previous version retained)".to_owned()
        })?;
        if !response.status().is_success() {
            return Err(format!(
                "plugin health check returned HTTP {}",
                response.status().as_u16()
            ));
        }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| "plugin health check read failure".to_owned())?;
            if bytes.len() + chunk.len() > 16 * 1024 {
                return Err("plugin health check response exceeds limit".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let health: Value = serde_json::from_slice(&bytes)
            .map_err(|_| "invalid plugin health check JSON".to_owned())?;
        if health.get("ok").and_then(Value::as_bool) != Some(true)
            || health.get("plugin_id").and_then(Value::as_str)
                != Some(candidate.manifest.id.as_str())
            || health.get("version").and_then(Value::as_str)
                != Some(candidate.manifest.version.as_str())
            || health.get("api_version").and_then(Value::as_u64)
                != Some(candidate.manifest.api_version as u64)
        {
            return Err("plugin health check identity or protocol mismatch".into());
        }
        Ok(())
    }
    pub fn redact_channel_config(
        &self,
        id: &str,
        config: &Map<String, Value>,
    ) -> Map<String, Value> {
        let store = self.store.read().unwrap();
        match store.entries.get(id) {
            Some(entry) => redact_entry_config(entry, config),
            None => config
                .keys()
                .map(|key| (key.clone(), Value::String("***".into())))
                .collect(),
        }
    }
    #[cfg(test)]
    pub fn register(&self, request: Registration) -> Result<Value, String> {
        let mut guard = self.store.write().unwrap();
        let version = self.candidate(&guard, request)?;
        self.publish_registration(&mut guard, version)
    }
    fn publish_registration(&self, guard: &mut Store, version: Version) -> Result<Value, String> {
        let id = version.manifest.id.clone();
        let revision = version.revision;
        let mut next = guard.clone();
        let entry = next.entries.entry(id.clone()).or_insert(Entry {
            enabled: true,
            revision,
            active: revision,
            versions: BTreeMap::new(),
        });
        entry.revision = revision;
        entry.active = revision;
        entry.versions.insert(revision, Arc::new(version));
        let leases = guard.lease_counts();
        let removable: Vec<_> = entry
            .versions
            .keys()
            .copied()
            .filter(|revision| *revision != entry.active)
            .filter(|revision| {
                leases
                    .get(&id)
                    .and_then(|counts| counts.get(revision))
                    .copied()
                    .unwrap_or(0)
                    == 0
            })
            .take(entry.versions.len().saturating_sub(3))
            .collect();
        let removed: Vec<_> = removable
            .into_iter()
            .filter_map(|revision| entry.versions.remove(&revision))
            .collect();
        self.publish(guard, next)?;
        if let Ok(root) = self.packages_root() {
            for version in removed {
                if let Some(package) = &version.managed {
                    if let Ok(path) = super::managed::package_path(&root, package) {
                        let _ = std::fs::remove_dir_all(path);
                    }
                }
            }
        }
        Ok(entry_view(&id, &guard.entries[&id], &guard.lease_counts()))
    }
    pub fn change(
        &self,
        id: &str,
        expected: u64,
        enabled: Option<bool>,
        rollback: Option<u64>,
    ) -> Result<Value, String> {
        let mut guard = self.store.write().unwrap();
        let entry = guard.entries.get(id).ok_or("plugin_not_found")?;
        if entry.revision != expected {
            return Err("revision_conflict".into());
        }
        let mut next = guard.clone();
        let entry = next.entries.get_mut(id).unwrap();
        let revision = expected.checked_add(1).ok_or("plugin revision exhausted")?;
        if let Some(target) = rollback {
            entry.versions.get(&target).ok_or("revision_not_found")?;
            entry.active = target;
        }
        if let Some(enabled) = enabled {
            entry.enabled = enabled;
        }
        entry.revision = revision;
        self.publish(&mut guard, next)?;
        Ok(entry_view(id, &guard.entries[id], &guard.lease_counts()))
    }
    pub fn list(&self) -> Value {
        let store = self.store.read().unwrap();
        let leases = store.lease_counts();
        let mut plugins: Vec<Value> = store
            .entries
            .iter()
            .map(|(id, e)| entry_view(id, e, &leases))
            .collect();
        plugins.extend(super::builtin::catalog());
        let jobs = self.jobs.lock().unwrap();
        for plugin in &mut plugins {
            if let Some(job) = plugin["id"].as_str().and_then(|id| jobs.get(id)) {
                plugin["last_error"] = json!(job.error);
                if job.updating {
                    plugin["status"] = json!("updating");
                }
            }
        }
        json!({"plugins":plugins,"trust":{"allowed_origins":self.config.trust.allowed_origins,"allow_loopback_http":self.config.trust.allow_loopback_http}})
    }
    pub fn interceptors(&self) -> Value {
        let store = self.store.read().unwrap();
        json!({"revision":store.interceptor_revision,"interceptors":*store.interceptors})
    }
    pub fn update_interceptors(
        &self,
        expected: u64,
        mut chain: Vec<HeaderInterceptor>,
    ) -> Result<Value, String> {
        validate_chain(&mut chain)?;
        let mut guard = self.store.write().unwrap();
        if expected != guard.interceptor_revision {
            return Err("revision_conflict".into());
        }
        let mut next = guard.clone();
        next.interceptor_revision = expected
            .checked_add(1)
            .ok_or("interceptor revision exhausted")?;
        next.interceptors = Arc::new(chain);
        self.publish(&mut guard, next)?;
        Ok(json!({"revision":guard.interceptor_revision,"interceptors":*guard.interceptors}))
    }
}

fn entry_view(id: &str, entry: &Entry, leases: &BTreeMap<String, BTreeMap<u64, usize>>) -> Value {
    let current = &entry.versions[&entry.active];
    json!({"id":id,"kind":if current.managed.is_some(){"managed"}else{"service"},"enabled":entry.enabled,"status":if !entry.enabled{"disabled"}else if current.managed.is_some() && current.runtime.as_ref().is_none_or(|runtime| !runtime.running()){"failed"}else{"running"},"revision":entry.revision,"installed_version":entry.versions.last_key_value().map(|(_, version)| version.manifest.version.as_str()),"active_version":current.manifest.version,"manifest":current.manifest,"endpoint":current.endpoint,"config":redact_entry_config(entry, &current.config),"service_auth":{"type":current.service_auth.kind,"token":if current.service_auth.token.is_empty(){""}else{"***"}},"requirements":current.managed.as_ref().map(|package| &package.requirements),"lease_count_kind":"snapshot_references","versions":entry.versions.values().map(|v| { let leases = leases.get(id).and_then(|versions| versions.get(&v.revision)).copied().unwrap_or(0); json!({"revision":v.revision,"version":v.manifest.version,"active":entry.active==v.revision,"leases":leases,"draining":leases>0 && (!entry.enabled || entry.active!=v.revision)}) }).collect::<Vec<_>>()})
}

fn redact_entry_config(entry: &Entry, config: &Map<String, Value>) -> Map<String, Value> {
    let current = &entry.versions[&entry.active];
    let mut redacted = current.manifest.redact(config);
    for key in config.keys() {
        if entry.versions.values().any(|version| {
            version
                .manifest
                .config_schema
                .iter()
                .any(|field| field.key == *key && field.secret)
        }) {
            redacted.insert(key.clone(), Value::String("***".into()));
        }
    }
    redacted
}
