//! Capture-only redaction. It never changes forwarded bytes or protocol state.
use crate::observability::usage::FactUsage;
use crate::plugins::PluginRegistry;
use crate::protocols::sse::{find_event_delimiter_from, parse_sse_frame, SseFrame};
use bytes::Bytes;
use serde::Serialize;
use serde_json::{json, Value};
use std::io::{self, Write};

pub(super) const MASK: &str = "[redacted]";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Json,
    Sse,
    Text,
    Unsupported,
}
pub(super) fn kind(content_type: &str, content_encoding: &str) -> Kind {
    if !content_encoding.trim().is_empty() && !content_encoding.eq_ignore_ascii_case("identity") {
        return Kind::Unsupported;
    }
    let value = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if value == "text/event-stream" {
        Kind::Sse
    } else if value == "application/json" || value.ends_with("+json") {
        Kind::Json
    } else if value.starts_with("text/")
        || matches!(
            value.as_str(),
            "image/svg+xml" | "application/xml" | "application/javascript" | "application/x-ndjson"
        )
    {
        Kind::Text
    } else {
        Kind::Unsupported
    }
}
pub(super) fn sensitive(name: &str) -> bool {
    let key = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    [
        "authorization",
        "proxyauthorization",
        "cookie",
        "setcookie",
        "apikey",
        "accesskey",
        "secretkey",
        "privatekey",
        "password",
        "passwd",
        "token",
        "accesstoken",
        "refreshtoken",
        "idtoken",
        "clientsecret",
        "secret",
        "awssecretaccesskey",
        "awsaccesskeyid",
        "awssessiontoken",
    ]
    .contains(&key.as_str())
        || key.ends_with("apikey")
        || key.ends_with("password")
        || key.ends_with("secret")
        || key.ends_with("token")
}
/// Geometric growth is clamped to the caller's capacity allowance, not merely
/// its current payload length. A caller sharing a budget across fields must
/// charge the returned Vec::capacity(), including unused spare bytes.
fn growth_target(
    len: usize,
    capacity: usize,
    additional: usize,
    limit: usize,
) -> Result<Option<usize>, ()> {
    let required = len
        .checked_add(additional)
        .filter(|required| *required <= limit)
        .ok_or(())?;
    if required <= capacity {
        return Ok(None);
    }
    Ok(Some(
        required
            .max(capacity.saturating_mul(2))
            .max(1024)
            .min(limit),
    ))
}
pub(super) fn reserve_bounded(
    bytes: &mut Vec<u8>,
    additional: usize,
    limit: usize,
) -> Result<(), ()> {
    if let Some(target) = growth_target(bytes.len(), bytes.capacity(), additional, limit)? {
        bytes
            .try_reserve_exact(target - bytes.len())
            .map_err(|_| ())?;
    }
    // Account actual capacities even on allocators which grant extra space.
    if bytes.capacity() > limit {
        *bytes = Vec::new();
        return Err(());
    }
    Ok(())
}
fn append_bounded(bytes: &mut Vec<u8>, value: &[u8], limit: usize) -> Result<(), ()> {
    reserve_bounded(bytes, value.len(), limit)?;
    bytes.extend_from_slice(value);
    Ok(())
}
fn bounded_text(text: &str, secrets: &[String], limit: usize) -> Result<String, ()> {
    let mut clean = String::new();
    let mut cursor = 0;
    // Cache each pattern's next hit (including permanent absence). Searches
    // advance monotonically instead of rescanning an absent long pattern over
    // every suffix when a short token repeats in a large capture.
    let mut hits = secrets
        .iter()
        .filter(|s| !s.is_empty())
        .map(|secret| (secret.as_str(), text.find(secret)))
        .collect::<Vec<_>>();
    while cursor < text.len() {
        let next = hits
            .iter()
            .filter_map(|(secret, offset)| offset.map(|offset| (offset, secret.len())))
            .min_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)));
        let (end, length) = next.unwrap_or((text.len(), 0));
        let addition = end - cursor + if length > 0 { MASK.len() } else { 0 };
        if clean.len().saturating_add(addition) > limit {
            return Err(());
        }
        if let Some(target) = growth_target(clean.len(), clean.capacity(), addition, limit)? {
            clean
                .try_reserve_exact(target - clean.len())
                .map_err(|_| ())?;
        }
        if clean.capacity() > limit {
            return Err(());
        }
        clean.push_str(&text[cursor..end]);
        if length > 0 {
            clean.push_str(MASK);
        }
        cursor = end + length;
        for (secret, offset) in &mut hits {
            if offset.is_some_and(|offset| offset < cursor) {
                *offset = text[cursor..].find(*secret).map(|offset| cursor + offset);
            }
        }
    }
    // Completed values relinquish spare capacity before the next field uses
    // the shared saved-content allowance.
    Ok(clean.into_boxed_str().into_string())
}
pub(super) fn clean_text(text: &str, secrets: &[String]) -> String {
    // Metadata and header scalars are bounded independently. Unlike repeated
    // String::replace, replacement markers are never scanned/replaced again.
    bounded_text(text, secrets, 16 * 1024).unwrap_or_else(|_| MASK.into())
}
fn clean_value(
    value: &mut Value,
    secrets: &[String],
    plugin: Option<(&PluginRegistry, &str)>,
    limit: usize,
) -> Result<(), ()> {
    if let Some((registry, id)) = plugin {
        if let Some(config) = value
            .pointer_mut("/channel/config")
            .and_then(Value::as_object_mut)
        {
            *config = registry.redact_channel_config(id, config);
        }
    }
    fn visit(value: &mut Value, secrets: &[String], remaining: &mut usize) -> Result<(), ()> {
        match value {
            Value::Object(map) => {
                let has_name = map.get("name").is_some_and(Value::is_string);
                let named_secret = map.get("name").and_then(Value::as_str).is_some_and(|name| {
                    sensitive(name) || crate::providers::parameters::header_sensitive(name, false)
                }) || map.get("secret").and_then(Value::as_bool) == Some(true);
                if has_name && named_secret && map.contains_key("value") {
                    map.insert("value".into(), json!(MASK));
                }
                for (key, value) in map {
                    if has_name && key == "secret" && value.is_boolean() {
                        continue;
                    }
                    if sensitive(key) {
                        *value = json!(MASK);
                    }
                    visit(value, secrets, remaining)?;
                }
            }
            Value::Array(items) => {
                for value in items {
                    visit(value, secrets, remaining)?;
                }
            }
            Value::String(text) => {
                let clean = if text == MASK || text == "***" {
                    text.clone()
                } else {
                    bounded_text(text, secrets, *remaining)?
                };
                if clean.len() > *remaining {
                    return Err(());
                }
                *remaining = remaining.saturating_sub(clean.len());
                *text = clean;
            }
            _ => {}
        }
        Ok(())
    }
    let mut remaining = limit;
    visit(value, secrets, &mut remaining)
}
struct Limited {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for Limited {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.bytes.len().saturating_add(bytes.len()) > self.limit {
            return Err(io::Error::other("redacted capture size limit"));
        }
        append_bounded(&mut self.bytes, bytes, self.limit)
            .map_err(|_| io::Error::other("capture allocation limit"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub(super) fn encode<T: Serialize + ?Sized>(value: &T, limit: usize) -> Result<Vec<u8>, ()> {
    let mut writer = Limited {
        bytes: Vec::new(),
        limit,
    };
    serde_json::to_writer(&mut writer, value).map_err(|_| ())?;
    Ok(writer.bytes.into_boxed_slice().into_vec())
}
#[derive(Default)]
pub(super) struct Observed {
    pub model: Option<String>,
    pub usage: FactUsage,
    pub terminal: Option<bool>,
}
impl Observed {
    fn inspect(&mut self, value: &Value, secrets: &[String]) {
        if let Some(model) = value
            .get("model")
            .or_else(|| value.pointer("/response/model"))
            .or_else(|| value.pointer("/message/model"))
            .and_then(Value::as_str)
        {
            self.model = Some(clean_text(
                &model.chars().take(512).collect::<String>(),
                secrets,
            ));
        }
        self.usage.merge(FactUsage::from_usage(
            value
                .get("usage")
                .or_else(|| value.pointer("/response/usage"))
                .or_else(|| value.pointer("/message/usage"))
                .or_else(|| value.get("usageMetadata")),
        ));
        let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
        if matches!(kind, "response.completed" | "message_stop")
            || value
                .pointer("/choices/0/finish_reason")
                .is_some_and(|v| !v.is_null())
        {
            self.terminal = Some(true);
        }
        if matches!(kind, "error" | "response.failed")
            || value.get("error").is_some_and(|v| !v.is_null())
        {
            self.terminal = Some(false);
        }
    }
}
pub(super) struct Redacted {
    pub data: Vec<u8>,
    pub state: &'static str,
    pub reason: Option<&'static str>,
    pub complete: bool,
    pub observed: Observed,
}
impl Redacted {
    fn unavailable(state: &'static str, reason: &'static str) -> Self {
        Self {
            data: vec![],
            state,
            reason: Some(reason),
            complete: false,
            observed: Observed::default(),
        }
    }
}
/// `limit` includes already-saved other fields; no unbounded redacted output.
pub(super) fn redact(
    kind: Kind,
    raw: &[u8],
    source_complete: bool,
    clipped: bool,
    limit: usize,
    secrets: &[String],
    plugin: Option<(&PluginRegistry, &str)>,
) -> Redacted {
    match kind {
        Kind::Unsupported => {
            Redacted::unavailable("unsupported", "unsupported_binary_multipart_or_encoding")
        }
        Kind::Json => {
            if clipped {
                return Redacted::unavailable("size_limit", "json_capture_exceeded_request_limit");
            }
            let Ok(mut value) = serde_json::from_slice::<Value>(raw) else {
                return Redacted::unavailable("redaction_failed", "json_not_complete_or_invalid");
            };
            let mut observed = Observed::default();
            observed.inspect(&value, secrets);
            if clean_value(&mut value, secrets, plugin, limit).is_err() {
                return Redacted::unavailable(
                    "size_limit",
                    "redacted_output_exceeded_request_limit",
                );
            }
            match encode(&value, limit) {
                Ok(data) => Redacted {
                    data,
                    state: if source_complete {
                        "available"
                    } else {
                        "truncated"
                    },
                    reason: (!source_complete).then_some("source_incomplete"),
                    complete: source_complete,
                    observed,
                },
                Err(()) => {
                    Redacted::unavailable("size_limit", "redacted_output_exceeded_request_limit")
                }
            }
        }
        Kind::Text => {
            if clipped || !source_complete {
                return Redacted::unavailable(
                    if clipped {
                        "size_limit"
                    } else {
                        "redaction_failed"
                    },
                    "incomplete_text_cannot_be_safely_redacted",
                );
            }
            let Ok(text) = std::str::from_utf8(raw) else {
                return Redacted::unavailable("unsupported", "unsupported_text_encoding");
            };
            // Structured text presented with a generic text MIME type is still
            // subjected to recursive key redaction, not mere token replacement.
            if let Ok(mut value) = serde_json::from_str::<Value>(text) {
                let mut observed = Observed::default();
                observed.inspect(&value, secrets);
                if clean_value(&mut value, secrets, plugin, limit).is_err() {
                    return Redacted::unavailable(
                        "size_limit",
                        "redacted_output_exceeded_request_limit",
                    );
                }
                return match encode(&value, limit) {
                    Ok(data) => Redacted {
                        data,
                        state: "available",
                        reason: None,
                        complete: true,
                        observed,
                    },
                    Err(()) => Redacted::unavailable(
                        "size_limit",
                        "redacted_output_exceeded_request_limit",
                    ),
                };
            }
            if text.trim_start().starts_with(['{', '[', '"']) {
                return Redacted::unavailable(
                    "redaction_failed",
                    "structured_text_is_not_valid_json",
                );
            }
            let Ok(clean) = bounded_text(text, secrets, limit) else {
                return Redacted::unavailable(
                    "size_limit",
                    "redacted_output_exceeded_request_limit",
                );
            };
            Redacted {
                data: clean.into_bytes(),
                state: "available",
                reason: None,
                complete: true,
                observed: Observed::default(),
            }
        }
        Kind::Sse => {
            let mut output = Vec::new();
            let mut offset = 0;
            let mut observed = Observed::default();
            let mut reason = None;
            while let Some((end, delimiter)) = find_event_delimiter_from(&raw[offset..], 0) {
                let frame = SseFrame {
                    wire: Bytes::copy_from_slice(&raw[offset..offset + end + delimiter]),
                    raw_len: end,
                    terminated: true,
                };
                let Ok(parsed) = parse_sse_frame(&frame) else {
                    reason = Some("sse_frame_redaction_failed");
                    break;
                };
                let mut clean = Vec::new();
                for line in parsed.raw.lines().filter(|line| !line.starts_with("data:")) {
                    let Ok(line) = bounded_text(
                        line,
                        secrets,
                        limit
                            .saturating_sub(output.len())
                            .saturating_sub(clean.len()),
                    ) else {
                        reason = Some("redacted_output_exceeded_request_limit");
                        break;
                    };
                    clean.extend_from_slice(line.as_bytes());
                    clean.push(b'\n');
                }
                if reason.is_some() {
                    break;
                }
                if let Some(data) = parsed.data {
                    let cleaned = if data.trim() == "[DONE]" {
                        observed.terminal = Some(true);
                        b"[DONE]".to_vec()
                    } else if let Ok(mut value) = serde_json::from_str::<Value>(&data) {
                        observed.inspect(&value, secrets);
                        if clean_value(
                            &mut value,
                            secrets,
                            plugin,
                            limit.saturating_sub(output.len()),
                        )
                        .is_err()
                        {
                            reason = Some("redacted_output_exceeded_request_limit");
                            break;
                        }
                        match encode(&value, limit.saturating_sub(output.len())) {
                            Ok(value) => value,
                            Err(()) => {
                                reason = Some("redacted_output_exceeded_request_limit");
                                break;
                            }
                        }
                    } else if data.trim_start().starts_with(['{', '[', '"']) {
                        reason = Some("sse_frame_redaction_failed");
                        break;
                    } else {
                        match bounded_text(&data, secrets, limit.saturating_sub(output.len())) {
                            Ok(text) => text.into_bytes(),
                            Err(()) => {
                                reason = Some("redacted_output_exceeded_request_limit");
                                break;
                            }
                        }
                    };
                    for line in cleaned.split(|b| *b == b'\n') {
                        clean.extend_from_slice(b"data: ");
                        clean.extend_from_slice(line);
                        clean.push(b'\n');
                    }
                }
                clean.push(b'\n');
                if output.len().saturating_add(clean.len()) > limit
                    || append_bounded(&mut output, &clean, limit).is_err()
                {
                    reason = Some("redacted_output_exceeded_request_limit");
                    break;
                }
                offset += end + delimiter;
            }
            let tail = raw[offset..].iter().any(|b| !b.is_ascii_whitespace());
            let complete = source_complete && !clipped && !tail && reason.is_none();
            let reason = reason.or(if clipped {
                Some("request_capture_size_limit")
            } else if tail {
                Some("unterminated_sse_frame_not_saved")
            } else if !source_complete {
                Some("source_incomplete")
            } else {
                None
            });
            let state = if complete {
                "available"
            } else if !output.is_empty() {
                "truncated"
            } else if clipped {
                "size_limit"
            } else {
                "redaction_failed"
            };
            Redacted {
                data: output.into_boxed_slice().into_vec(),
                state,
                reason,
                complete,
                observed,
            }
        }
    }
}

pub(super) fn usage_json(usage: &FactUsage) -> Option<Value> {
    if [
        usage.input,
        usage.output,
        usage.cache_read,
        usage.cache_write,
        usage.cache_write_1h,
    ]
    .iter()
    .all(Option::is_none)
    {
        return None;
    }
    Some(
        json!({"input_tokens":usage.input,"output_tokens":usage.output,"total_tokens":usage.input.zip(usage.output).map(|(a,b)|a.saturating_add(b)),"cache_read_tokens":usage.cache_read,"cache_write_tokens":usage.cache_write,"cache_write_1h_tokens":usage.cache_write_1h}),
    )
}
