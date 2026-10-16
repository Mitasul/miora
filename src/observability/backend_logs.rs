use serde::Serialize;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

pub(crate) const CAPACITY: usize = 2000;

#[derive(Clone, Serialize)]
pub(crate) struct Entry {
    pub(crate) id: u64,
    pub(crate) at_ms: u128,
    pub(crate) level: String,
    pub(crate) target: String,
    pub(crate) message: String,
    pub(crate) details: Value,
}

#[derive(Default)]
struct Buffer {
    next_id: u64,
    entries: VecDeque<Entry>,
}

static BUFFER: OnceLock<Mutex<Buffer>> = OnceLock::new();

fn buffer() -> &'static Mutex<Buffer> {
    BUFFER.get_or_init(|| Mutex::new(Buffer::default()))
}

pub(crate) fn event(value: Value) {
    eprintln!("{value}");
    let status = value
        .get("http_status")
        .or_else(|| value.get("status_code"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let mut level = value
        .get("severity")
        .and_then(Value::as_str)
        .unwrap_or_else(|| {
            if value.get("error").is_some() || status >= 500 {
                "ERROR"
            } else if status >= 400 {
                "WARN"
            } else {
                "INFO"
            }
        })
        .to_ascii_uppercase();
    if level == "WARNING" {
        level = "WARN".into();
    }
    let target = value
        .get("event_type")
        .or_else(|| value.get("event"))
        .and_then(Value::as_str)
        .unwrap_or("gateway");
    let text = value
        .get("error")
        .and_then(Value::as_str)
        .or_else(|| value.get("message").and_then(Value::as_str))
        .unwrap_or(target);
    let cleaned = crate::observability::request_trace::error_fields(
        &serde_json::to_vec(&json!({"error":{"message":text}})).expect("log message"),
        &[],
    );
    let mut details = json!({});
    for name in [
        "request_id",
        "trace_id",
        "provider",
        "model",
        "http_status",
        "status_code",
        "duration_ms",
        "api_key_count",
        "provider_count",
        "input_tokens",
        "output_tokens",
        "total_tokens",
        "upstream_attempt_count",
    ] {
        if let Some(field) = value.get(name) {
            details[name] = field.clone();
        }
    }
    let mut buffer = buffer().lock().expect("backend log buffer");
    buffer.next_id += 1;
    let id = buffer.next_id;
    if buffer.entries.len() == CAPACITY {
        buffer.entries.pop_front();
    }
    buffer.entries.push_back(Entry {
        id,
        at_ms: crate::runtime::clock::unix_millis(),
        level,
        target: target.chars().take(160).collect(),
        message: cleaned["error_message"].as_str().unwrap_or("").to_owned(),
        details,
    });
}

pub(crate) fn read(after: u64, level: &str, search: &str, limit: usize) -> Value {
    let buffer = buffer().lock().expect("backend log buffer");
    let search = search.to_lowercase();
    let mut items: Vec<_> = buffer
        .entries
        .iter()
        .rev()
        .filter(|entry| {
            entry.id > after
                && (level.is_empty() || entry.level == level)
                && (search.is_empty()
                    || format!("{} {} {}", entry.target, entry.message, entry.details)
                        .to_lowercase()
                        .contains(&search))
        })
        .take(limit.min(500))
        .cloned()
        .collect();
    items.reverse();
    json!({"items":items,"max_id":buffer.next_id,"oldest_id":buffer.entries.front().map(|entry| entry.id),"buffer_size":buffer.entries.len(),"capacity":CAPACITY})
}
