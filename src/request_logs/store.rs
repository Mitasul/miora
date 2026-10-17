use super::{CaptureField, CapturedBlob, LogRecord, LogSettings, Shared};
use axum::http::StatusCode;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

#[cfg(test)]
mod tests;

#[derive(Debug)]
pub(super) struct Failure {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}
impl Failure {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }
    pub fn disabled() -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "request_logs_disabled",
            "Request-log storage is disabled by DISABLE_DATABASE",
        )
    }
    pub fn unavailable() -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "request_logs_unavailable",
            "Request-log storage is unavailable",
        )
    }
    pub fn busy() -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "request_logs_busy",
            "Request-log writer is busy; retry later",
        )
    }
    pub fn bad(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_log_request", message)
    }
}
impl From<rusqlite::Error> for Failure {
    fn from(_: rusqlite::Error) -> Self {
        Self::unavailable()
    }
}
impl From<std::io::Error> for Failure {
    fn from(_: std::io::Error) -> Self {
        Self::unavailable()
    }
}

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Filters {
    pub request_id: Option<String>,
    pub channel: Option<String>,
    pub model: Option<String>,
    pub status: Option<u16>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub ids: Option<Vec<String>>,
}
impl Filters {
    pub fn sql(&self) -> (String, Vec<rusqlite::types::Value>) {
        let mut clauses = Vec::new();
        let mut values = Vec::new();
        for (column, value) in [("request_id", &self.request_id), ("channel", &self.channel)] {
            if let Some(value) = value {
                clauses.push(format!("r.{column} = ?"));
                values.push(value.clone().into());
            }
        }
        if let Some(model) = &self.model {
            clauses
                .push("(r.requested_model = ? OR r.sent_model = ? OR r.response_model = ?)".into());
            for _ in 0..3 {
                values.push(model.clone().into());
            }
        }
        if let Some(status) = self.status {
            clauses.push("r.status = ?".into());
            values.push(i64::from(status).into());
        }
        if let Some(at) = self.from_ms {
            clauses.push("r.at_ms >= ?".into());
            values.push(at.into());
        }
        if let Some(at) = self.to_ms {
            clauses.push("r.at_ms < ?".into());
            values.push(at.into());
        }
        if let Some(ids) = &self.ids {
            if ids.is_empty() {
                clauses.push("0".into());
            } else {
                clauses.push(format!("r.id IN ({})", vec!["?"; ids.len()].join(",")));
                for id in ids {
                    values.push(id.clone().into());
                }
            }
        }
        (
            if clauses.is_empty() {
                "1".into()
            } else {
                clauses.join(" AND ")
            },
            values,
        )
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum CleanupAction {
    ClearFields,
    DeleteRows,
}
#[derive(Debug, Deserialize)]
pub(super) struct CleanupRequest {
    pub dry_run: bool,
    pub action: Option<CleanupAction>,
    #[serde(default)]
    pub fields: Vec<CaptureField>,
    #[serde(default)]
    pub filters: Filters,
    pub confirmation_token: Option<String>,
}

pub(super) enum Operation {
    UpdateSettings {
        revision: u64,
        settings: Value,
    },
    List {
        filters: Filters,
        page: u64,
        page_size: u64,
    },
    Detail(String),
    Content {
        id: String,
        field: CaptureField,
        attempt: Option<u32>,
        limit: Option<usize>,
    },
    Cleanup(CleanupRequest),
}
pub(super) enum Reply {
    Json(Value),
    Content {
        bytes: Vec<u8>,
        info: Value,
        truncated: bool,
    },
}
pub(super) enum Command {
    Begin(Box<LogRecord>),
    Finish(Box<LogRecord>, usize),
    Fail(String, String),
    Query(
        Operation,
        tokio::sync::oneshot::Sender<Result<Reply, Failure>>,
    ),
}
pub(super) struct Database {
    pub conn: Connection,
    pub directory: PathBuf,
}

impl Database {
    pub fn open(directory: PathBuf) -> Result<Self, String> {
        std::fs::create_dir_all(directory.join("bodies"))
            .map_err(|_| "Create request-log directory failed")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
                .map_err(|_| "Protect request-log directory failed")?;
        }
        let conn = Connection::open(directory.join("index.sqlite"))
            .map_err(|_| "Open request-log index failed")?;
        conn.busy_timeout(Duration::from_secs(2))
            .map_err(|_| "Configure request-log index failed")?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS log_settings(id INTEGER PRIMARY KEY CHECK(id=1),revision INTEGER NOT NULL,config_json TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS request_log_rows(id TEXT PRIMARY KEY,request_id TEXT NOT NULL,at_ms INTEGER NOT NULL,channel TEXT,requested_model TEXT,sent_model TEXT,response_model TEXT,status INTEGER,active INTEGER NOT NULL,summary_json TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS request_log_time ON request_log_rows(at_ms DESC,id DESC);
            CREATE INDEX IF NOT EXISTS request_log_request_id ON request_log_rows(request_id);
            CREATE TABLE IF NOT EXISTS request_log_fields(record_id TEXT NOT NULL REFERENCES request_log_rows(id) ON DELETE CASCADE,attempt INTEGER NOT NULL,field TEXT NOT NULL,observed_bytes INTEGER NOT NULL,saved_bytes INTEGER NOT NULL,complete INTEGER NOT NULL,capture_state TEXT NOT NULL,availability TEXT NOT NULL,reason TEXT,availability_reason TEXT,content_type TEXT,representation TEXT,file_path TEXT,expires_at_ms INTEGER,PRIMARY KEY(record_id,attempt,field));
            CREATE INDEX IF NOT EXISTS request_log_expiry ON request_log_fields(availability,expires_at_ms);
            CREATE TABLE IF NOT EXISTS cleanup_plans(token TEXT PRIMARY KEY,expires_at_ms INTEGER NOT NULL,action TEXT NOT NULL,fields_json TEXT NOT NULL,filters_json TEXT NOT NULL,matched_rows INTEGER NOT NULL,active_excluded INTEGER NOT NULL,estimated_bytes INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS cleanup_targets(token TEXT NOT NULL REFERENCES cleanup_plans(token) ON DELETE CASCADE,record_id TEXT NOT NULL,PRIMARY KEY(token,record_id));")
            .map_err(|_| "Initialize request-log index failed")?;
        conn.execute(
            "INSERT OR IGNORE INTO log_settings(id,revision,config_json) VALUES(1,1,?)",
            [serde_json::to_string(&LogSettings::default()).unwrap()],
        )
        .map_err(|_| "Initialize request-log settings failed")?;
        let db = Self { conn, directory };
        db.recover()
            .map_err(|_| "Recover request-log index failed")?;
        Ok(db)
    }
    pub fn load_settings(&self) -> Result<(u64, LogSettings), String> {
        let (revision, config): (i64, String) = self
            .conn
            .query_row(
                "SELECT revision,config_json FROM log_settings WHERE id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| "Read request-log settings failed")?;
        let settings: LogSettings =
            serde_json::from_str(&config).map_err(|_| "Invalid persisted request-log settings")?;
        settings.validate()?;
        Ok((revision as u64, settings))
    }
    fn recover(&self) -> Result<(), Failure> {
        let ids: Vec<String> = self
            .conn
            .prepare("SELECT id FROM request_log_rows WHERE active=1")?
            .query_map([], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        for id in ids {
            self.fail(&id, "process_restarted")?;
        }
        let live: std::collections::BTreeSet<String> = self.conn.prepare("SELECT file_path FROM request_log_fields WHERE file_path IS NOT NULL AND availability='available'")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
        for entry in std::fs::read_dir(self.directory.join("bodies"))? {
            let entry = entry?;
            let relative = format!("bodies/{}", entry.file_name().to_string_lossy());
            if entry.file_type()?.is_file() && !live.contains(&relative) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        Ok(())
    }
    fn begin(&self, record: &LogRecord) -> Result<(), Failure> {
        // Begin has no filesystem side effects: publish its row and eight
        // pending fields in one WAL transaction, with no partial initial row.
        let transaction = self.conn.unchecked_transaction()?;
        transaction.prepare_cached("INSERT OR IGNORE INTO request_log_rows(id,request_id,at_ms,channel,requested_model,sent_model,response_model,status,active,summary_json) VALUES(?,?,?,?,?,?,?,?,1,?)")?.execute(params![record.id,record.request_id,record.at_ms,record.channel,record.requested_model,record.sent_model,record.response_model,record.status,redact_json(record.summary(true)).to_string()])?;
        {
            let mut fields = transaction.prepare_cached("INSERT OR IGNORE INTO request_log_fields(record_id,attempt,field,observed_bytes,saved_bytes,complete,capture_state,availability) VALUES(?,-1,?,0,0,0,'pending','pending')")?;
            for field in CaptureField::ALL {
                fields.execute(params![record.id, field.as_str()])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }
    fn exists(&self, id: &str) -> Result<bool, Failure> {
        Ok(self
            .conn
            .prepare_cached("SELECT 1 FROM request_log_rows WHERE id=?")?
            .query_row([id], |_| Ok(()))
            .optional()?
            .is_some())
    }
    fn finish(&self, record: &LogRecord, settings: &LogSettings, now: i64) -> Result<(), Failure> {
        let active: Option<bool> = self
            .conn
            .prepare_cached("SELECT active FROM request_log_rows WHERE id=?")?
            .query_row([&record.id], |r| r.get(0))
            .optional()?;
        if active != Some(true) {
            return Ok(());
        }
        let mut write_failed = false;
        for field in CaptureField::ALL {
            let default = CapturedBlob {
                capture_state: "not_captured".into(),
                reason:
                    Some(
                        if field.as_str().starts_with("upstream_")
                            && !record.attempt_blobs.is_empty()
                        {
                            "stored_per_attempt"
                        } else {
                            "not_selected"
                        }
                        .into(),
                    ),
                ..Default::default()
            };
            if self
                .write_blob(
                    record,
                    -1,
                    field,
                    record.blobs.get(&field).unwrap_or(&default),
                    settings,
                    now,
                )
                .is_err()
            {
                write_failed = true;
            }
        }
        for (attempt, fields) in &record.attempt_blobs {
            for (field, blob) in fields {
                if self
                    .write_blob(record, i64::from(*attempt), *field, blob, settings, now)
                    .is_err()
                {
                    write_failed = true;
                }
            }
        }
        let mut summary = redact_json(record.summary(false));
        if write_failed {
            summary["capture_failure"] = json!("storage_write_failed");
            summary["incomplete"] = json!(true);
        }
        // Terminal records update an existing row only. Deleted rows cannot reappear.
        self.conn.prepare_cached("UPDATE request_log_rows SET channel=?,requested_model=?,sent_model=?,response_model=?,status=?,active=0,summary_json=? WHERE id=? AND active=1")?.execute(params![record.channel,record.requested_model,record.sent_model,record.response_model,record.status,summary.to_string(),record.id])?;
        self.conn.prepare_cached("UPDATE request_log_fields SET capture_state='storage_write_failed',availability='unavailable',reason='storage_write_failed' WHERE record_id=? AND availability='pending'")?.execute([&record.id])?;
        if write_failed {
            return Err(Failure::unavailable());
        }
        Ok(())
    }
    fn fail(&self, id: &str, reason: &str) -> Result<(), Failure> {
        let summary: Option<String> = self
            .conn
            .query_row(
                "SELECT summary_json FROM request_log_rows WHERE id=? AND active=1",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(summary) = summary {
            let mut value: Value =
                serde_json::from_str(&summary).unwrap_or_else(|_| json!({"id":id}));
            value["active"] = json!(false);
            value["incomplete"] = json!(true);
            value["capture_failure"] = json!(reason);
            self.conn.execute(
                "UPDATE request_log_rows SET active=0,summary_json=? WHERE id=? AND active=1",
                params![value.to_string(), id],
            )?;
            self.conn.execute("UPDATE request_log_fields SET capture_state=?,availability='unavailable',reason=? WHERE record_id=? AND availability='pending'", params![reason,reason,id])?;
        }
        Ok(())
    }
    fn write_blob(
        &self,
        record: &LogRecord,
        attempt: i64,
        field: CaptureField,
        blob: &CapturedBlob,
        settings: &LogSettings,
        now: i64,
    ) -> Result<(), Failure> {
        let mut capture_state = if blob.capture_state.is_empty() {
            "not_captured".to_owned()
        } else {
            blob.capture_state.clone()
        };
        let mut reason = blob.reason.clone();
        let mut availability = "not_captured";
        let mut availability_reason: Option<&str> = None;
        let mut path = None;
        let mut saved_bytes = 0u64;
        let mut complete = blob.complete;
        let raw_allowed = record.body_expires_at_ms.is_some();
        let content_present = !blob.data.is_empty() || capture_state == "captured";
        if !raw_allowed {
            availability_reason = Some("body_retention_disabled");
        } else if content_present
            && record
                .body_expires_at_ms
                .is_some_and(|expiry| expiry <= now)
        {
            availability = "expired";
            availability_reason = Some("retention_expired");
        } else if content_present && capture_state != "unsupported" {
            match content_bytes(field, blob) {
                Some(data) => {
                    if data.len() > 16 * 1024 * 1024 {
                        capture_state = "truncated".into();
                        complete = false;
                        reason = Some("store_record_limit".into());
                    } else if data.len() as u64 > settings.limits.total_body_bytes {
                        availability = "evicted";
                        availability_reason = Some("total_body_limit");
                    } else {
                        self.evict_to_fit(settings.limits.total_body_bytes, data.len() as u64)?;
                        let relative =
                            format!("bodies/{}-{}-{}.bin", record.id, attempt, field.as_str());
                        let destination = self.directory.join(&relative);
                        let mut options = std::fs::OpenOptions::new();
                        options.write(true).create_new(true);
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::OpenOptionsExt;
                            options.mode(0o600);
                        }
                        let mut file = options.open(&destination)?;
                        use std::io::Write;
                        if file.write_all(&data).is_err() {
                            drop(file);
                            let _ = std::fs::remove_file(destination);
                            return Err(Failure::unavailable());
                        }
                        saved_bytes = data.len() as u64;
                        path = Some(relative);
                        availability = "available";
                    }
                }
                None => {
                    capture_state = "unsupported".into();
                    complete = false;
                    reason = Some("unsupported_or_invalid_text_capture".into());
                }
            }
        }
        if availability != "available" {
            complete = false;
        }
        let result = self.conn.prepare_cached("INSERT INTO request_log_fields(record_id,attempt,field,observed_bytes,saved_bytes,complete,capture_state,availability,reason,availability_reason,content_type,representation,file_path,expires_at_ms) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(record_id,attempt,field) DO UPDATE SET observed_bytes=excluded.observed_bytes,saved_bytes=excluded.saved_bytes,complete=excluded.complete,capture_state=excluded.capture_state,availability=excluded.availability,reason=excluded.reason,availability_reason=excluded.availability_reason,content_type=excluded.content_type,representation=excluded.representation,file_path=excluded.file_path,expires_at_ms=excluded.expires_at_ms").and_then(|mut statement| statement.execute(params![record.id,attempt,field.as_str(),blob.observed_bytes.min(i64::MAX as u64) as i64,saved_bytes as i64,complete,capture_state,availability,reason,availability_reason,blob.content_type,blob.representation,path,record.body_expires_at_ms]));
        if result.is_err() {
            if let Some(path) = path {
                let _ = std::fs::remove_file(self.directory.join(path));
            }
        }
        result?;
        Ok(())
    }
    fn settings_value(&self, shared: &Shared) -> Value {
        let snapshot = shared.settings.read().unwrap();
        let mut settings = serde_json::to_value(&snapshot.settings).unwrap();
        if !snapshot.runtime_rotation_managed {
            settings.as_object_mut().unwrap().remove("runtime");
        }
        json!({"revision":snapshot.revision,"settings":settings,"backend_enabled":snapshot.backend_enabled,"runtime_rotation_managed":snapshot.runtime_rotation_managed,"capture_fields":CaptureField::ALL})
    }
    fn update_settings(
        &self,
        revision: u64,
        mut value: Value,
        shared: &Shared,
    ) -> Result<Value, Failure> {
        let previous = shared.settings.read().unwrap().clone();
        if revision != previous.revision {
            return Err(Failure::new(
                StatusCode::CONFLICT,
                "log_settings_conflict",
                "Log settings changed; reload before saving",
            ));
        }
        if !value.is_object() {
            return Err(Failure::bad("settings must be an object"));
        }
        if !previous.runtime_rotation_managed && value.get("runtime").is_some() {
            return Err(Failure::bad(
                "Runtime log rotation is not managed for this instance",
            ));
        }
        if value.get("runtime").is_none() {
            value["runtime"] = serde_json::to_value(&previous.settings.runtime).unwrap();
        }
        let settings: LogSettings = serde_json::from_value(value)
            .map_err(|_| Failure::bad("Invalid log settings fields or types"))?;
        settings.validate().map_err(Failure::bad)?;
        let next = revision.checked_add(1).ok_or_else(Failure::unavailable)?;
        let changed = self.conn.execute(
            "UPDATE log_settings SET revision=?,config_json=? WHERE id=1 AND revision=?",
            params![
                next as i64,
                serde_json::to_string(&settings).unwrap(),
                revision as i64
            ],
        )?;
        if changed == 0 {
            return Err(Failure::new(
                StatusCode::CONFLICT,
                "log_settings_conflict",
                "Log settings changed; reload before saving",
            ));
        }
        {
            let mut snapshot = shared.settings.write().unwrap();
            snapshot.settings = settings.clone();
            snapshot.revision = next;
        }
        self.evict_to_fit(settings.limits.total_body_bytes, 0)?;
        Ok(self.settings_value(shared))
    }
    fn list(&self, filters: Filters, page: u64, page_size: u64) -> Result<Value, Failure> {
        let (clause, values) = filters.sql();
        let total: i64 = self.conn.query_row(
            &format!("SELECT COUNT(*) FROM request_log_rows r WHERE {clause}"),
            params_from_iter(values.iter()),
            |r| r.get(0),
        )?;
        let mut values = values;
        values.push((page_size as i64).into());
        values.push(
            (page
                .saturating_sub(1)
                .saturating_mul(page_size)
                .min(i64::MAX as u64) as i64)
                .into(),
        );
        let mut statement = self.conn.prepare(&format!("SELECT summary_json,active FROM request_log_rows r WHERE {clause} ORDER BY at_ms DESC,id DESC LIMIT ? OFFSET ?"))?;
        let rows = statement
            .query_map(params_from_iter(values.iter()), |r| {
                let raw: String = r.get(0)?;
                let active: bool = r.get(1)?;
                let mut value: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
                value["active"] = json!(active);
                Ok(value)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({"rows":rows,"total":total,"page":page,"page_size":page_size}))
    }
    fn detail(&self, id: &str) -> Result<Value, Failure> {
        let row: Option<(String, bool)> = self
            .conn
            .query_row(
                "SELECT summary_json,active FROM request_log_rows WHERE id=?",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (raw, active) = row.ok_or_else(|| {
            Failure::new(
                StatusCode::NOT_FOUND,
                "request_log_not_found",
                "Request log not found",
            )
        })?;
        let mut record: Value = serde_json::from_str(&raw).map_err(|_| Failure::unavailable())?;
        record["active"] = json!(active);
        let fields = self.conn.prepare("SELECT attempt,field,observed_bytes,saved_bytes,complete,capture_state,availability,reason,availability_reason,content_type,representation,expires_at_ms FROM request_log_fields WHERE record_id=? ORDER BY attempt,field")?
            .query_map([id], field_info)?.collect::<Result<Vec<_>, _>>()?;
        Ok(json!({"record":record,"fields":fields,"active":active}))
    }
    fn content(
        &self,
        id: &str,
        field: CaptureField,
        attempt: Option<u32>,
        limit: Option<usize>,
        now: i64,
    ) -> Result<Reply, Failure> {
        let attempt = attempt.map(i64::from).unwrap_or(-1);
        let row: Option<(Value,Option<String>)> = self.conn.query_row("SELECT attempt,field,observed_bytes,saved_bytes,complete,capture_state,availability,reason,availability_reason,content_type,representation,expires_at_ms,file_path FROM request_log_fields WHERE record_id=? AND attempt=? AND field=?", params![id,attempt,field.as_str()], |r| Ok((field_info(r)?,r.get(12)?))).optional()?;
        let (info, path) = row.ok_or_else(|| {
            Failure::new(
                StatusCode::NOT_FOUND,
                "log_content_not_found",
                "No captured field exists at this record/attempt",
            )
        })?;
        if matches!(info["availability"].as_str(), Some("available" | "expired"))
            && info["expires_at_ms"]
                .as_i64()
                .is_some_and(|expiry| expiry <= now)
        {
            self.clear_one(id, attempt, field.as_str(), "expired", "retention_expired")?;
            return Err(Failure::new(
                StatusCode::GONE,
                "log_content_expired",
                "Captured content has expired",
            ));
        }
        if info["availability"] != "available" {
            return Err(Failure::new(
                StatusCode::GONE,
                "log_content_unavailable",
                "Captured content is not available; inspect its availability and reason",
            ));
        }
        let path = path.ok_or_else(Failure::unavailable)?;
        let mut file = match std::fs::File::open(self.directory.join(path)) {
            Ok(file) => file,
            Err(_) => {
                self.conn.execute("UPDATE request_log_fields SET availability='unavailable',availability_reason='file_missing',file_path=NULL WHERE record_id=? AND attempt=? AND field=?", params![id,attempt,field.as_str()])?;
                return Err(Failure::new(
                    StatusCode::GONE,
                    "log_content_unavailable",
                    "Captured file is unavailable",
                ));
            }
        };
        use std::io::Read;
        let length = file.metadata()?.len();
        let take = limit.unwrap_or(16 * 1024 * 1024);
        let mut bytes = Vec::new();
        (&mut file).take(take as u64).read_to_end(&mut bytes)?;
        Ok(Reply::Content {
            truncated: length > bytes.len() as u64,
            bytes,
            info,
        })
    }
    fn operate(&self, operation: Operation, shared: &Shared) -> Result<Reply, Failure> {
        let value = match operation {
            Operation::UpdateSettings { revision, settings } => {
                self.update_settings(revision, settings, shared)?
            }
            Operation::List {
                filters,
                page,
                page_size,
            } => self.list(filters, page, page_size)?,
            Operation::Detail(id) => self.detail(&id)?,
            Operation::Content {
                id,
                field,
                attempt,
                limit,
            } => return self.content(&id, field, attempt, limit, super::now_ms()),
            Operation::Cleanup(input) => self.cleanup(input, super::now_ms())?,
        };
        Ok(Reply::Json(value))
    }
}

fn field_info(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let attempt: i64 = row.get(0)?;
    let mut availability: String = row.get(6)?;
    let expires: Option<i64> = row.get(11)?;
    let mut availability_reason: Option<String> = row.get(8)?;
    if availability == "available" && expires.is_some_and(|expiry| expiry <= super::now_ms()) {
        availability = "expired".into();
        availability_reason = Some("retention_expired".into());
    }
    Ok(
        json!({"attempt":if attempt < 0 { None } else { Some(attempt) },"field":row.get::<_,String>(1)?,"observed_bytes":row.get::<_,i64>(2)?,"saved_bytes":row.get::<_,i64>(3)?,"complete":row.get::<_,bool>(4)?,"capture_state":row.get::<_,String>(5)?,"available":availability=="available","availability":availability,"reason":row.get::<_,Option<String>>(7)?,"availability_reason":availability_reason,"content_type":row.get::<_,Option<String>>(9)?,"representation":row.get::<_,Option<String>>(10)?,"expires_at_ms":expires,"redacted":true}),
    )
}

fn content_bytes(field: CaptureField, blob: &CapturedBlob) -> Option<Vec<u8>> {
    let mime = blob
        .content_type
        .as_deref()
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if field.is_headers() || mime == "application/json" || mime.ends_with("+json") {
        if blob.data.is_empty() {
            return Some(Vec::new());
        }
        let value: Value = serde_json::from_slice(&blob.data).ok()?;
        return serde_json::to_vec(&redact_json(value)).ok();
    }
    if !(mime.is_empty() || mime.starts_with("text/") || mime == "image/svg+xml") {
        return None;
    }
    std::str::from_utf8(&blob.data).ok()?;
    // Text/SSE has already been safely sanitized by the capture boundary.
    Some(blob.data.clone())
}
fn redact_json(value: Value) -> Value {
    match value {
        Value::Object(mut map) => {
            let named_secret = map
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(secret_name)
                || map.get("secret") == Some(&Value::Bool(true));
            if named_secret && map.contains_key("value") {
                map.insert("value".into(), json!("[REDACTED]"));
            }
            Value::Object(
                map.into_iter()
                    .map(|(name, value)| {
                        let redacted =
                            if secret_name(&name) && !(name == "secret" && value.is_boolean()) {
                                json!("[REDACTED]")
                            } else {
                                redact_json(value)
                            };
                        (name, redacted)
                    })
                    .collect(),
            )
        }
        Value::Array(values) => Value::Array(values.into_iter().map(redact_json).collect()),
        other => other,
    }
}
fn secret_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase().replace('-', "_");
    matches!(
        name.as_str(),
        "api"
            | "api_key"
            | "apikey"
            | "authorization"
            | "proxy_authorization"
            | "cookie"
            | "set_cookie"
            | "password"
            | "secret"
            | "token"
            | "access_token"
            | "refresh_token"
            | "id_token"
            | "client_secret"
            | "private_key"
            | "aws_secret_key"
            | "aws_session_token"
    ) || name.ends_with("_api_key")
}

pub(super) fn run(
    database: Database,
    receiver: mpsc::Receiver<Command>,
    finals: mpsc::Receiver<Command>,
    shared: Arc<Shared>,
) {
    struct Alive(Arc<Shared>);
    impl Drop for Alive {
        fn drop(&mut self) {
            self.0.worker_alive.store(false, Ordering::Relaxed);
        }
    }
    let _alive = Alive(shared.clone());
    let mut pending = BTreeMap::<String, Command>::new();
    let mut last_gc = Instant::now();
    let process = |command: Command, pending: &mut BTreeMap<String, Command>| match command {
        Command::Begin(record) => {
            let id = record.id.clone();
            if database.begin(&record).is_err() {
                shared.write_failures.fetch_add(1, Ordering::Relaxed);
                shared.active.lock().unwrap().remove(&id);
            }
            if let Some(command) = pending.remove(&id) {
                complete(&database, command, &shared);
            }
        }
        command @ (Command::Finish(_, _) | Command::Fail(_, _)) => {
            let id = match &command {
                Command::Finish(record, _) => record.id.clone(),
                Command::Fail(id, _) => id.clone(),
                _ => unreachable!(),
            };
            if database.exists(&id).unwrap_or(true) {
                // A read error is a storage failure, not an out-of-order Begin.
                complete(&database, command, &shared);
            } else if shared.active.lock().unwrap().contains_key(&id) {
                pending.insert(id, command);
            } else if let Command::Finish(_, bytes) = command {
                shared.queued_bytes.fetch_sub(bytes, Ordering::AcqRel);
            }
        }
        Command::Query(operation, reply) => {
            let _ = reply.send(database.operate(operation, &shared));
        }
    };
    loop {
        for command in finals.try_iter() {
            process(command, &mut pending);
        }
        match receiver.recv_timeout(Duration::from_millis(250)) {
            Ok(command) => process(command, &mut pending),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                for command in finals.try_iter() {
                    process(command, &mut pending);
                }
                break;
            }
        }
        if last_gc.elapsed() >= Duration::from_secs(30) {
            let settings = shared.settings.read().unwrap().settings.clone();
            if database.gc(&settings, super::now_ms()).is_err() {
                shared.write_failures.fetch_add(1, Ordering::Relaxed);
            }
            last_gc = Instant::now();
        }
    }
    shared.worker_alive.store(false, Ordering::Relaxed);
}
fn complete(database: &Database, command: Command, shared: &Shared) {
    let (id, result) = match command {
        Command::Finish(record, bytes) => {
            let settings = shared.settings.read().unwrap().settings.clone();
            let result = database.finish(&record, &settings, super::now_ms());
            let id = record.id.clone();
            drop(record); // Release capture memory only after file/SQLite work completes.
            shared.queued_bytes.fetch_sub(bytes, Ordering::AcqRel);
            (id, result)
        }
        Command::Fail(id, reason) => {
            let result = database.fail(&id, &reason);
            (id, result)
        }
        _ => return,
    };
    if result.is_err() {
        shared.write_failures.fetch_add(1, Ordering::Relaxed);
        let _ = database.fail(&id, "storage_write_failed");
    }
    shared.completed.fetch_add(1, Ordering::Relaxed);
    shared.active.lock().unwrap().remove(&id);
}
