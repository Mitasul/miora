use super::*;
use crate::request_logs::config::{LogLimits, RetentionMode, RowRetention};
use crate::request_logs::{Inner, RequestLogs, SettingsSnapshot};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize};
use std::sync::{Mutex, RwLock};

#[test]
fn begin_is_atomic_when_a_pending_field_insert_fails() {
    let temp = Temp::new();
    let db = Database::open(temp.0.join("logs")).unwrap();
    let record = record("atomic-begin", "", None);
    db.conn.execute_batch("CREATE TRIGGER reject_pending BEFORE INSERT ON request_log_fields WHEN NEW.field='request_body' BEGIN SELECT RAISE(ABORT,'fixture pending failure'); END;").unwrap();
    assert!(db.begin(&record).is_err());
    assert!(!db.exists(&record.id).unwrap());
    assert_eq!(
        db.conn
            .query_row("SELECT COUNT(*) FROM request_log_fields", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    db.conn
        .execute_batch("DROP TRIGGER reject_pending")
        .unwrap();
    db.begin(&record).unwrap();
    assert!(db.exists(&record.id).unwrap());
    assert_eq!(
        db.conn
            .query_row("SELECT COUNT(*) FROM request_log_fields", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        8
    );
}

#[test]
fn cached_field_write_failure_removes_the_new_body_file() {
    let temp = Temp::new();
    let db = Database::open(temp.0.join("logs")).unwrap();
    let now = crate::request_logs::now_ms();
    let first = record("failed-field", "fixture body", Some(now + 3_600_000));
    db.begin(&first).unwrap();
    db.conn.execute_batch("CREATE TRIGGER reject_saved BEFORE INSERT ON request_log_fields WHEN NEW.field='response_body' AND NEW.saved_bytes>0 BEGIN SELECT RAISE(ABORT,'fixture field failure'); END;").unwrap();
    assert!(db.finish(&first, &LogSettings::default(), now).is_err());
    assert_eq!(
        std::fs::read_dir(db.directory.join("bodies"))
            .unwrap()
            .count(),
        0
    );
    let active: bool = db
        .conn
        .query_row(
            "SELECT active FROM request_log_rows WHERE id=?",
            [&first.id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!active);
    db.conn.execute_batch("DROP TRIGGER reject_saved").unwrap();
    let second = record("after-failure", "second body", Some(now + 3_600_000));
    db.begin(&second).unwrap();
    db.finish(&second, &LogSettings::default(), now).unwrap();
    assert_eq!(
        std::fs::read_dir(db.directory.join("bodies"))
            .unwrap()
            .count(),
        1
    );
}

struct Temp(std::path::PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "miora-request-log-test-{}",
            crate::request_logs::random_token().unwrap()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn record(id: &str, data: &str, expires: Option<i64>) -> LogRecord {
    let mut record = LogRecord {
        id: id.into(),
        request_id: format!("caller-{id}"),
        at_ms: crate::request_logs::now_ms(),
        status: Some(200),
        requested_model: Some("alias".into()),
        upstream_complete: Some(true),
        relay_complete: Some(true),
        body_expires_at_ms: expires,
        ..Default::default()
    };
    record.blobs.insert(
        CaptureField::ResponseBody,
        CapturedBlob {
            data: data.as_bytes().to_vec(),
            observed_bytes: data.len() as u64,
            complete: true,
            capture_state: "captured".into(),
            content_type: Some("text/plain".into()),
            representation: Some("relay_entity_offered_to_http_stack".into()),
            ..Default::default()
        },
    );
    record
}
fn finish(db: &Database, record: &LogRecord, settings: &LogSettings) {
    db.begin(record).unwrap();
    db.finish(record, settings, crate::request_logs::now_ms())
        .unwrap();
}
fn field(db: &Database, id: &str) -> Value {
    db.detail(id).unwrap()["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["field"] == "response_body" && field["attempt"].is_null())
        .unwrap()
        .clone()
}
fn cleanup(dry_run: bool, action: Option<CleanupAction>, token: Option<String>) -> CleanupRequest {
    CleanupRequest {
        dry_run,
        action,
        fields: vec![CaptureField::ResponseBody],
        filters: Filters::default(),
        confirmation_token: token,
    }
}

#[test]
fn zero_hours_keeps_metadata_without_content_or_download() {
    let temp = Temp::new();
    let db = Database::open(temp.0.join("logs")).unwrap();
    let record = record("zero", "content-that-must-not-be-saved", None);
    finish(&db, &record, &LogSettings::default());
    assert_eq!(db.detail("zero").unwrap()["record"]["status"], 200);
    let info = field(&db, "zero");
    assert_eq!(info["available"], false);
    assert_eq!(info["saved_bytes"], 0);
    assert_eq!(info["complete"], false);
    assert_eq!(
        std::fs::read_dir(temp.0.join("logs/bodies"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(
        db.content(
            "zero",
            CaptureField::ResponseBody,
            None,
            None,
            crate::request_logs::now_ms()
        )
        .err()
        .unwrap()
        .status,
        StatusCode::GONE
    );
}
#[test]
fn expiry_is_visible_before_gc_and_summary_survives() {
    let temp = Temp::new();
    let db = Database::open(temp.0.join("logs")).unwrap();
    let now = crate::request_logs::now_ms();
    finish(
        &db,
        &record("expires", "full text", Some(now + 3_600_000)),
        &LogSettings::default(),
    );
    assert_eq!(field(&db, "expires")["available"], true);
    db.conn.execute("UPDATE request_log_fields SET expires_at_ms=? WHERE record_id='expires' AND field='response_body'",[now-1]).unwrap();
    let info = field(&db, "expires");
    assert_eq!(info["availability"], "expired");
    assert_eq!(info["available"], false);
    assert_eq!(info["capture_state"], "captured");
    assert_eq!(info["complete"], true);
    assert_eq!(
        std::fs::read_dir(temp.0.join("logs/bodies"))
            .unwrap()
            .count(),
        1
    );
    db.gc(&LogSettings::default(), now).unwrap();
    assert_eq!(db.detail("expires").unwrap()["record"]["status"], 200);
    assert_eq!(
        std::fs::read_dir(temp.0.join("logs/bodies"))
            .unwrap()
            .count(),
        0
    );
}
#[test]
fn quota_evicts_oldest_content_and_keeps_rows() {
    let temp = Temp::new();
    let db = Database::open(temp.0.join("logs")).unwrap();
    let now = crate::request_logs::now_ms();
    let settings = LogSettings {
        limits: LogLimits {
            per_request_bytes: 1024,
            total_body_bytes: 12,
        },
        ..Default::default()
    };
    let mut first = record("first", "old-content", Some(now + 3_600_000));
    first.at_ms = now - 1000;
    finish(&db, &first, &settings);
    finish(
        &db,
        &record("second", "new-content", Some(now + 3_600_000)),
        &settings,
    );
    assert_eq!(field(&db, "first")["availability"], "evicted");
    assert_eq!(field(&db, "first")["capture_state"], "captured");
    assert_eq!(field(&db, "second")["available"], true);
    assert_eq!(db.list(Filters::default(), 1, 50).unwrap()["total"], 2);
    let mut zero = settings.clone();
    zero.limits.total_body_bytes = 0;
    db.gc(&zero, now).unwrap();
    assert_eq!(field(&db, "second")["availability"], "evicted");
}
#[test]
fn cleanup_freezes_targets_excludes_active_and_preserves_statistics() {
    let temp = Temp::new();
    let db = Database::open(temp.0.join("logs")).unwrap();
    let now = crate::request_logs::now_ms();
    let stats = Connection::open(temp.0.join("stats.db")).unwrap();
    stats
        .execute_batch(
            "CREATE TABLE request_stats(id INTEGER); INSERT INTO request_stats VALUES(1);",
        )
        .unwrap();
    let original = record("old", "old content", Some(now + 3_600_000));
    finish(&db, &original, &LogSettings::default());
    db.begin(&record("active", "later completion", Some(now + 3_600_000)))
        .unwrap();
    let plan = db
        .cleanup(cleanup(true, Some(CleanupAction::ClearFields), None), now)
        .unwrap();
    assert_eq!(plan["matched_rows"], 1);
    assert_eq!(plan["active_excluded"], 1);
    assert_eq!(field(&db, "old")["available"], true);
    finish(
        &db,
        &record("new", "new content", Some(now + 3_600_000)),
        &LogSettings::default(),
    );
    let token = plan["confirmation_token"].as_str().unwrap().to_owned();
    let result = db
        .cleanup(cleanup(false, None, Some(token.clone())), now)
        .unwrap();
    assert_eq!(result["affected_rows"], 1);
    assert_eq!(field(&db, "old")["availability"], "cleared");
    assert_eq!(field(&db, "new")["available"], true);
    assert_eq!(db.detail("old").unwrap()["record"]["status"], 200);
    assert!(db.cleanup(cleanup(false, None, Some(token)), now).is_err());
    let mut deletion = cleanup(true, Some(CleanupAction::DeleteRows), None);
    deletion.filters.ids = Some(vec!["old".into()]);
    let plan = db.cleanup(deletion, now).unwrap();
    db.cleanup(
        cleanup(
            false,
            None,
            Some(plan["confirmation_token"].as_str().unwrap().into()),
        ),
        now,
    )
    .unwrap();
    // A terminal UPDATE for a deleted row must never recreate the summary/body.
    db.finish(&original, &LogSettings::default(), now).unwrap();
    assert!(db.detail("old").is_err());
    assert_eq!(
        stats
            .query_row("SELECT COUNT(*) FROM request_stats", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn secrets_are_redacted_and_attempt_content_has_real_availability() {
    let temp = Temp::new();
    let db = Database::open(temp.0.join("logs")).unwrap();
    let now = crate::request_logs::now_ms();
    let mut record = record("secrets", "safe", Some(now + 3_600_000));
    let value=br#"{"Authorization":"Bearer never-store-this","Cookie":"session=private","ordinary":"visible"}"#;
    record.attempt_blobs.entry(2).or_default().insert(
        CaptureField::UpstreamRequestHeaders,
        CapturedBlob {
            data: value.to_vec(),
            observed_bytes: value.len() as u64,
            complete: true,
            capture_state: "captured".into(),
            content_type: Some("application/json".into()),
            ..Default::default()
        },
    );
    finish(&db, &record, &LogSettings::default());
    let Reply::Content {
        bytes,
        info,
        truncated,
    } = db
        .content(
            "secrets",
            CaptureField::UpstreamRequestHeaders,
            Some(2),
            None,
            now,
        )
        .unwrap()
    else {
        panic!("content expected")
    };
    let text = String::from_utf8(bytes).unwrap();
    assert!(!text.contains("never-store-this"));
    assert!(!text.contains("session=private"));
    assert!(text.contains("visible"));
    assert!(!truncated);
    assert_eq!(info["attempt"], 2);
    let mut plan = cleanup(true, Some(CleanupAction::ClearFields), None);
    plan.fields = vec![CaptureField::UpstreamRequestHeaders];
    let plan = db.cleanup(plan, now).unwrap();
    db.cleanup(
        cleanup(
            false,
            None,
            Some(plan["confirmation_token"].as_str().unwrap().into()),
        ),
        now,
    )
    .unwrap();
    assert_eq!(
        db.content(
            "secrets",
            CaptureField::UpstreamRequestHeaders,
            Some(2),
            None,
            now
        )
        .err()
        .unwrap()
        .status,
        StatusCode::GONE
    );
}
#[test]
fn row_retention_only_deletes_new_log_rows() {
    let temp = Temp::new();
    let db = Database::open(temp.0.join("logs")).unwrap();
    let now = crate::request_logs::now_ms();
    let mut old = record("old", "old", Some(now + 3_600_000));
    old.at_ms = now - 3 * 86_400_000;
    finish(&db, &old, &LogSettings::default());
    db.gc(&LogSettings::default(), now).unwrap();
    assert!(db.detail("old").is_ok());
    let settings = LogSettings {
        row_retention: RowRetention {
            mode: RetentionMode::Days,
            days: 1,
        },
        ..Default::default()
    };
    db.gc(&settings, now).unwrap();
    assert!(db.detail("old").is_err());
}
fn shared() -> Arc<Shared> {
    Arc::new(Shared {
        settings: RwLock::new(SettingsSnapshot {
            revision: 1,
            settings: LogSettings::default(),
            backend_enabled: true,
            runtime_rotation_managed: false,
        }),
        active: Mutex::new(BTreeMap::new()),
        queued_bytes: AtomicUsize::new(0),
        dropped: AtomicU64::new(0),
        write_failures: AtomicU64::new(0),
        completed: AtomicU64::new(0),
        worker_alive: AtomicBool::new(true),
        sequence: AtomicU64::new(0),
        prefix: "test".into(),
        initialization_error: None,
    })
}
#[tokio::test]
async fn saturated_final_queue_finishes_even_when_it_arrives_before_begin() {
    let temp = Temp::new();
    let db = Database::open(temp.0.join("logs")).unwrap();
    let shared = shared();
    let (sender, receiver) = mpsc::sync_channel(64);
    let (final_sender, final_receiver) = mpsc::sync_channel(super::super::ACTIVE_LIMIT);
    let logs = RequestLogs {
        inner: Arc::new(Inner {
            sender: Some(sender),
            final_sender: Some(final_sender),
            shared: shared.clone(),
            worker: None,
        }),
    };
    let now = crate::request_logs::now_ms();
    assert!(logs.begin(record("queued", "unused-start-body", Some(now + 3_600_000))));
    shared
        .queued_bytes
        .store(super::super::QUEUED_BYTES_LIMIT, Ordering::Relaxed);
    assert!(!logs.enqueue(record(
        "queued",
        "must-drop-final-body",
        Some(now + 3_600_000)
    )));
    // Both queues are filled before the worker starts: terminal fallback is read first.
    let worker_shared = shared.clone();
    let worker = std::thread::spawn(move || run(db, receiver, final_receiver, worker_shared));
    let reply = tokio::time::timeout(
        Duration::from_secs(3),
        logs.query(Operation::Detail("queued".into())),
    )
    .await
    .unwrap()
    .unwrap();
    let Reply::Json(value) = reply else {
        panic!("JSON expected")
    };
    assert_eq!(value["active"], false);
    assert_eq!(value["record"]["capture_failure"], "queue_full");
    assert_eq!(value["record"]["status"], 200);
    assert_eq!(shared.active.lock().unwrap().len(), 0);
    assert_eq!(
        std::fs::read_dir(temp.0.join("logs/bodies"))
            .unwrap()
            .count(),
        0
    );
    shared.queued_bytes.store(0, Ordering::Relaxed);
    drop(logs);
    worker.join().unwrap();
}
#[tokio::test]
async fn storage_initialization_failure_has_an_explicit_non_capturing_stub() {
    let temp = Temp::new();
    let path = temp.0.join("not-a-directory");
    std::fs::write(&path, b"keep").unwrap();
    let reason = RequestLogs::open(path.clone(), true, false).err().unwrap();
    let logs = RequestLogs::unavailable(reason);
    assert!(!logs.settings().backend_enabled);
    assert!(logs.status()["initialization_error"].is_string());
    assert!(!logs.begin(record("unavailable", "not saved", None)));
    let failure = logs
        .query(Operation::List {
            filters: Filters::default(),
            page: 1,
            page_size: 50,
        })
        .await
        .err()
        .unwrap();
    assert_eq!(failure.code, "request_logs_unavailable");
    assert_eq!(std::fs::read(path).unwrap(), b"keep");
}

#[tokio::test]
async fn settings_cas_and_restart_restore() {
    let temp = Temp::new();
    let path = temp.0.join("logs");
    let logs = RequestLogs::open(path.clone(), true, true).unwrap();
    let original = logs.settings();
    let mut settings = original.settings.clone();
    settings.body_retention_hours = 2;
    settings.capture_fields = vec![CaptureField::RequestBody];
    settings.runtime.retained_files = 4;
    logs.query(Operation::UpdateSettings {
        revision: original.revision,
        settings: serde_json::to_value(&settings).unwrap(),
    })
    .await
    .unwrap();
    let failure = logs
        .query(Operation::UpdateSettings {
            revision: original.revision,
            settings: serde_json::to_value(&settings).unwrap(),
        })
        .await
        .err()
        .unwrap();
    assert_eq!(failure.status, StatusCode::CONFLICT);
    drop(logs);
    let reopened = RequestLogs::open(path, true, true).unwrap();
    assert_eq!(reopened.settings().settings, settings);
    assert_eq!(reopened.settings().revision, original.revision + 1);
}
