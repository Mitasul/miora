//! Independent bounded request-log storage. Capture/production hooks live in capture.rs.
pub(crate) mod api;
pub(crate) mod capture;
mod config;
mod retention;
mod store;
pub(crate) use config::{LogSettings, SettingsSnapshot};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) const MAX_CAPTURE_BYTES_GLOBAL: usize = 32 * 1024 * 1024;
const QUEUED_BYTES_LIMIT: usize = 32 * 1024 * 1024;
const ACTIVE_LIMIT: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CaptureField {
    RequestHeaders,
    RequestBody,
    UpstreamRequestHeaders,
    UpstreamRequestBody,
    UpstreamResponseHeaders,
    UpstreamResponseBody,
    ResponseHeaders,
    ResponseBody,
}
impl CaptureField {
    pub const ALL: [Self; 8] = [
        Self::RequestHeaders,
        Self::RequestBody,
        Self::UpstreamRequestHeaders,
        Self::UpstreamRequestBody,
        Self::UpstreamResponseHeaders,
        Self::UpstreamResponseBody,
        Self::ResponseHeaders,
        Self::ResponseBody,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RequestHeaders => "request_headers",
            Self::RequestBody => "request_body",
            Self::UpstreamRequestHeaders => "upstream_request_headers",
            Self::UpstreamRequestBody => "upstream_request_body",
            Self::UpstreamResponseHeaders => "upstream_response_headers",
            Self::UpstreamResponseBody => "upstream_response_body",
            Self::ResponseHeaders => "response_headers",
            Self::ResponseBody => "response_body",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|field| field.as_str() == value)
    }
    pub fn is_headers(self) -> bool {
        self.as_str().ends_with("_headers")
    }
}

#[derive(Debug, Default)]
pub(crate) struct CapturedBlob {
    pub data: Vec<u8>,
    pub observed_bytes: u64,
    pub complete: bool,
    pub capture_state: String,
    pub reason: Option<String>,
    pub content_type: Option<String>,
    pub representation: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct LogRecord {
    pub id: String,
    pub request_id: String,
    pub method: Option<String>,
    pub endpoint: Option<String>,
    pub at_ms: i64,
    pub channel: Option<String>,
    pub requested_model: Option<String>,
    pub sent_model: Option<String>,
    pub response_model: Option<String>,
    pub status: Option<u16>,
    pub duration_ms: Option<f64>,
    pub first_byte_ms: Option<f64>,
    pub first_text_ms: Option<f64>,
    pub usage: Option<Value>,
    pub attempts: Vec<Value>,
    pub attempt_count: u64,
    pub omitted_attempts: u64,
    pub cancelled: bool,
    pub stream: bool,
    pub incomplete: bool,
    pub error_stage: Option<String>,
    pub capture_failure: Option<String>,
    pub upstream_complete: Option<bool>,
    pub relay_complete: Option<bool>,
    pub body_expires_at_ms: Option<i64>,
    pub blobs: BTreeMap<CaptureField, CapturedBlob>,
    pub attempt_blobs: BTreeMap<u32, BTreeMap<CaptureField, CapturedBlob>>,
    pub capture_budget: Option<capture::CaptureReservation>,
}
impl LogRecord {
    fn raw_bytes(&self) -> usize {
        self.blobs
            .values()
            .chain(
                self.attempt_blobs
                    .values()
                    .flat_map(|fields| fields.values()),
            )
            .map(|blob| blob.data.len())
            .sum()
    }
    fn discard_content(&mut self, reason: &str) {
        for blob in self.blobs.values_mut().chain(
            self.attempt_blobs
                .values_mut()
                .flat_map(|fields| fields.values_mut()),
        ) {
            blob.data = Vec::new();
            blob.capture_state = reason.into();
            blob.complete = false;
            blob.reason = Some(reason.into());
        }
        self.capture_failure = Some(reason.into());
        self.incomplete = true;
        // The lease also covers metadata; retain it until the terminal writer/drop.
    }
    fn summary(&self, active: bool) -> Value {
        json!({"id":self.id,"request_id":self.request_id,"method":self.method,"endpoint":self.endpoint,"at_ms":self.at_ms,"channel":self.channel,
            "requested_model":self.requested_model,"sent_model":self.sent_model,"response_model":self.response_model,
            "status":self.status,"duration_ms":self.duration_ms,"first_byte_ms":self.first_byte_ms,"first_text_ms":self.first_text_ms,
            "usage":self.usage,"attempts":self.attempts,"attempt_count":self.attempt_count.max(self.attempts.len() as u64),"omitted_attempts":self.omitted_attempts,"cancelled":self.cancelled,
            "stream":self.stream,"incomplete":self.incomplete,"error_stage":self.error_stage,"capture_failure":self.capture_failure,
            "upstream_complete":self.upstream_complete,"relay_complete":self.relay_complete,"active":active})
    }
}

#[derive(Clone)]
pub(crate) struct RequestLogs {
    inner: Arc<Inner>,
}
struct Inner {
    sender: Option<mpsc::SyncSender<store::Command>>,
    final_sender: Option<mpsc::SyncSender<store::Command>>,
    shared: Arc<Shared>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Inner {
    fn drop(&mut self) {
        self.sender.take();
        self.final_sender.take();
        if let Some(worker) = self.worker.take() {
            if worker.thread().id() != std::thread::current().id() {
                let _ = worker.join();
            }
        }
    }
}
struct Shared {
    settings: RwLock<SettingsSnapshot>,
    active: Mutex<BTreeMap<String, bool>>,
    queued_bytes: AtomicUsize,
    dropped: AtomicU64,
    write_failures: AtomicU64,
    completed: AtomicU64,
    worker_alive: AtomicBool,
    sequence: AtomicU64,
    prefix: String,
    initialization_error: Option<String>,
}
impl RequestLogs {
    pub fn from_env(enabled: bool) -> Result<Self, String> {
        let directory =
            std::env::var("MIORA_REQUEST_LOG_DIR").unwrap_or_else(|_| "data/request-logs".into());
        let runtime_managed =
            std::env::var("MIORA_RUNTIME_LOG_MANAGED").is_ok_and(|value| value == "1");
        Self::open(PathBuf::from(directory), enabled, runtime_managed)
    }
    pub fn unavailable(reason: impl Into<String>) -> Self {
        let managed = std::env::var("MIORA_RUNTIME_LOG_MANAGED").is_ok_and(|value| value == "1");
        let mut logs =
            Self::open(PathBuf::new(), false, managed).expect("disabled log store requires no I/O");
        let inner = Arc::get_mut(&mut logs.inner).unwrap();
        Arc::get_mut(&mut inner.shared)
            .unwrap()
            .initialization_error = Some(reason.into());
        logs
    }

    fn open(directory: PathBuf, enabled: bool, runtime_managed: bool) -> Result<Self, String> {
        let database = if enabled {
            Some(store::Database::open(directory)?)
        } else {
            None
        };
        let (revision, settings) = match &database {
            Some(db) => db.load_settings()?,
            None => (0, LogSettings::default()),
        };
        let shared = Arc::new(Shared {
            settings: RwLock::new(SettingsSnapshot {
                revision,
                settings,
                backend_enabled: enabled,
                runtime_rotation_managed: runtime_managed,
            }),
            active: Mutex::new(BTreeMap::new()),
            queued_bytes: AtomicUsize::new(0),
            dropped: AtomicU64::new(0),
            write_failures: AtomicU64::new(0),
            completed: AtomicU64::new(0),
            worker_alive: AtomicBool::new(enabled),
            sequence: AtomicU64::new(0),
            initialization_error: None,
            prefix: if enabled {
                random_token()?
            } else {
                "disabled".into()
            },
        });
        if let Some(database) = database {
            let (sender, receiver) = mpsc::sync_channel(64);
            // One terminal message per reserved active slot, independent of raw-body queue pressure.
            let (final_sender, final_receiver) = mpsc::sync_channel(ACTIVE_LIMIT);
            let worker_shared = shared.clone();
            let worker = std::thread::Builder::new()
                .name("miora-request-logs".into())
                .spawn(move || store::run(database, receiver, final_receiver, worker_shared))
                .map_err(|_| "Could not start request-log writer")?;
            Ok(Self {
                inner: Arc::new(Inner {
                    sender: Some(sender),
                    final_sender: Some(final_sender),
                    shared,
                    worker: Some(worker),
                }),
            })
        } else {
            Ok(Self {
                inner: Arc::new(Inner {
                    sender: None,
                    final_sender: None,
                    shared,
                    worker: None,
                }),
            })
        }
    }
    pub fn settings(&self) -> SettingsSnapshot {
        self.inner.shared.settings.read().unwrap().clone()
    }
    pub fn new_id(&self) -> String {
        format!(
            "{}-{:x}",
            self.inner.shared.prefix,
            self.inner.shared.sequence.fetch_add(1, Ordering::Relaxed)
        )
    }
    pub fn begin(&self, mut record: LogRecord) -> bool {
        let snapshot = self.settings();
        if !snapshot.backend_enabled
            || !snapshot.settings.enabled
            || !self.inner.shared.worker_alive.load(Ordering::Relaxed)
        {
            return false;
        }
        if record.id.is_empty()
            || record.id.len() > 128
            || !record
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return false;
        }
        record.blobs.clear();
        record.attempt_blobs.clear();
        record.capture_budget = None;
        {
            let mut active = self.inner.shared.active.lock().unwrap();
            if active.len() >= ACTIVE_LIMIT || active.contains_key(&record.id) {
                self.inner.shared.dropped.fetch_add(1, Ordering::Relaxed);
                return false;
            }
            active.insert(record.id.clone(), false);
        }
        let id = record.id.clone();
        if self
            .inner
            .sender
            .as_ref()
            .unwrap()
            .try_send(store::Command::Begin(Box::new(record)))
            .is_err()
        {
            self.inner.shared.active.lock().unwrap().remove(&id);
            self.inner.shared.dropped.fetch_add(1, Ordering::Relaxed);
            false
        } else {
            true
        }
    }
    pub fn enqueue(&self, mut record: LogRecord) -> bool {
        if !self.claim_final(&record.id) {
            return false;
        }
        let bytes = record.raw_bytes();
        let reserved = self
            .inner
            .shared
            .queued_bytes
            .fetch_update(Ordering::AcqRel, Ordering::Relaxed, |current| {
                current
                    .checked_add(bytes)
                    .filter(|total| *total <= QUEUED_BYTES_LIMIT)
            })
            .is_ok();
        if reserved {
            match self
                .inner
                .sender
                .as_ref()
                .unwrap()
                .try_send(store::Command::Finish(Box::new(record), bytes))
            {
                Ok(()) => return true,
                Err(mpsc::TrySendError::Full(store::Command::Finish(returned, _)))
                | Err(mpsc::TrySendError::Disconnected(store::Command::Finish(returned, _))) => {
                    record = *returned;
                }
                Err(_) => unreachable!("finish command variant"),
            }
            self.inner
                .shared
                .queued_bytes
                .fetch_sub(bytes, Ordering::AcqRel);
        }
        self.inner.shared.dropped.fetch_add(1, Ordering::Relaxed);
        record.discard_content("queue_full");
        let id = record.id.clone();
        if self
            .inner
            .final_sender
            .as_ref()
            .unwrap()
            .try_send(store::Command::Finish(Box::new(record), 0))
            .is_err()
        {
            self.inner.shared.active.lock().unwrap().remove(&id);
            self.inner
                .shared
                .write_failures
                .fetch_add(1, Ordering::Relaxed);
        }
        false
    }
    pub fn fail(&self, id: &str, reason: &str) -> bool {
        if !self.claim_final(id) {
            return false;
        }
        let result = self
            .inner
            .final_sender
            .as_ref()
            .unwrap()
            .try_send(store::Command::Fail(id.into(), reason.into()))
            .is_ok();
        if !result {
            self.inner.shared.active.lock().unwrap().remove(id);
        }
        result
    }
    fn claim_final(&self, id: &str) -> bool {
        if self.inner.sender.is_none() || !self.inner.shared.worker_alive.load(Ordering::Relaxed) {
            return false;
        }
        let mut active = self.inner.shared.active.lock().unwrap();
        match active.get_mut(id) {
            Some(done) if !*done => {
                *done = true;
                true
            }
            _ => false,
        }
    }
    async fn query(&self, operation: store::Operation) -> Result<store::Reply, store::Failure> {
        let sender = self.inner.sender.as_ref().ok_or_else(|| {
            if let Some(reason) = &self.inner.shared.initialization_error {
                store::Failure::new(
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    "request_logs_unavailable",
                    reason.clone(),
                )
            } else {
                store::Failure::disabled()
            }
        })?;
        let (reply, receiver) = tokio::sync::oneshot::channel();
        sender
            .try_send(store::Command::Query(operation, reply))
            .map_err(|_| store::Failure::busy())?;
        receiver.await.map_err(|_| store::Failure::unavailable())?
    }
    fn status(&self) -> Value {
        let shared = &self.inner.shared;
        json!({"active_records":shared.active.lock().unwrap().len(),"queued_bytes":shared.queued_bytes.load(Ordering::Relaxed),"dropped_content_records":shared.dropped.load(Ordering::Relaxed),"write_failures":shared.write_failures.load(Ordering::Relaxed),"completed_records":shared.completed.load(Ordering::Relaxed),"worker_alive":shared.worker_alive.load(Ordering::Relaxed),"initialization_error":shared.initialization_error})
    }
}
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
fn random_token() -> Result<String, String> {
    use ring::rand::SecureRandom;
    let mut bytes = [0; 16];
    ring::rand::SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| "Request-log secure randomness unavailable")?;
    Ok(hex::encode(bytes))
}
