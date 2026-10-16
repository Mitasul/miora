use crate::config::snapshot::Provider;
use crate::providers::parameters::{profile, Profile};
use serde_json::{json, Value};

pub(crate) fn validate(preferences: &Value) -> Result<(), String> {
    if let Some(mode) = preferences.get("reasoning_history") {
        if !matches!(mode.as_str(), Some("preserve" | "strip")) {
            return Err("reasoning_history must be preserve or strip".into());
        }
    }
    if let Some(retry) = preferences.pointer("/response_policy/retry_empty") {
        if !retry.is_boolean() {
            return Err("response_policy.retry_empty must be boolean".into());
        }
    }
    if let Some(suffixes) = preferences.get("reasoning_suffixes") {
        let suffixes = suffixes
            .as_object()
            .ok_or("reasoning_suffixes must be an object")?;
        if suffixes.len() > 16 {
            return Err("At most 16 reasoning suffixes are supported per channel".into());
        }
        for (suffix, policy) in suffixes {
            if !suffix.starts_with('-') || suffix.len() < 2 || suffix.len() > 32 {
                return Err("Reasoning suffixes must start with '-' and contain 2-32 bytes".into());
            }
            if !policy.is_object()
                || policy.get("effort").is_some_and(|effort| {
                    !matches!(effort.as_str(), Some("low" | "medium" | "high"))
                })
                || policy
                    .get("budget_tokens")
                    .is_some_and(|budget| !budget.as_u64().is_some_and(|budget| budget >= 1024))
            {
                return Err("Reasoning suffix policy accepts effort (low/medium/high) and budget_tokens >= 1024".into());
            }
        }
    }
    if preferences
        .get("allow_byok")
        .is_some_and(|value| !value.is_boolean())
    {
        return Err("allow_byok must be boolean".into());
    }
    Ok(())
}

pub(crate) fn retry_empty(provider: &Provider) -> bool {
    provider
        .preferences
        .get("response_policy")
        .and_then(|policy| policy.get("retry_empty"))
        .and_then(Value::as_bool)
        == Some(true)
}

pub(crate) fn blocked(value: &Value) -> bool {
    value
        .pointer("/promptFeedback/blockReason")
        .is_some_and(|reason| !reason.is_null())
        || matches!(
            value
                .pointer("/candidates/0/finishReason")
                .and_then(Value::as_str),
            Some("SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT")
        )
        || value
            .pointer("/choices/0/finish_reason")
            .and_then(Value::as_str)
            == Some("content_filter")
        || value
            .pointer("/choices/0/message/refusal")
            .and_then(Value::as_str)
            .is_some_and(|text| !text.is_empty())
        || value
            .pointer("/choices/0/delta/refusal")
            .and_then(Value::as_str)
            .is_some_and(|text| !text.is_empty())
}

pub(crate) fn has_output(value: &Value) -> bool {
    if blocked(value) {
        return true;
    }
    let text = |value: Option<&Value>| {
        value
            .and_then(Value::as_str)
            .is_some_and(|text| !text.is_empty())
    };
    text(value.pointer("/choices/0/message/content"))
        || text(value.pointer("/choices/0/message/reasoning_content"))
        || value
            .pointer("/choices/0/message/tool_calls")
            .and_then(Value::as_array)
            .is_some_and(|calls| !calls.is_empty())
        || value
            .get("content")
            .and_then(Value::as_array)
            .is_some_and(|blocks| {
                blocks.iter().any(|block| {
                    text(block.get("text"))
                        || text(block.get("thinking"))
                        || block["type"] == "tool_use"
                })
            })
        || value
            .get("candidates")
            .and_then(Value::as_array)
            .is_some_and(|candidates| {
                candidates.iter().any(|candidate| {
                    candidate
                        .pointer("/content/parts")
                        .and_then(Value::as_array)
                        .is_some_and(|parts| {
                            parts.iter().any(|part| {
                                text(part.get("text"))
                                    || part.get("functionCall").is_some()
                                    || part.get("inlineData").is_some()
                            })
                        })
                })
            })
}

pub(crate) fn apply(
    provider: &Provider,
    model: &str,
    path: &str,
    body: &mut Value,
) -> Result<(), String> {
    if provider
        .preferences
        .get("reasoning_history")
        .and_then(Value::as_str)
        == Some("strip")
    {
        if let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) {
            for message in messages {
                if let Some(object) = message.as_object_mut() {
                    object.remove("reasoning_content");
                    object.remove("reasoning");
                }
                if let Some(blocks) = message.get_mut("content").and_then(Value::as_array_mut) {
                    blocks.retain(|block| {
                        !matches!(
                            block["type"].as_str(),
                            Some("thinking" | "redacted_thinking")
                        )
                    });
                }
            }
        }
        if let Some(contents) = body.get_mut("contents").and_then(Value::as_array_mut) {
            for content in contents {
                if let Some(parts) = content.get_mut("parts").and_then(Value::as_array_mut) {
                    parts.retain(|part| part["thought"].as_bool() != Some(true));
                }
            }
        }
    }
    let Some(suffixes) = provider
        .preferences
        .get("reasoning_suffixes")
        .and_then(Value::as_object)
    else {
        return Ok(());
    };
    let Some((_, policy)) = suffixes
        .iter()
        .filter(|(suffix, _)| model.ends_with(suffix.as_str()))
        .max_by_key(|(suffix, _)| suffix.len())
    else {
        return Ok(());
    };
    match profile(provider, path) {
        Profile::Chat => {
            if body.get("reasoning_effort").is_none() {
                if let Some(effort) = policy.get("effort") {
                    body["reasoning_effort"] = effort.clone();
                }
            }
        }
        Profile::Responses | Profile::Codex => {
            if body.get("reasoning").is_none() {
                if let Some(effort) = policy.get("effort") {
                    body["reasoning"] = json!({"effort":effort});
                }
            }
        }
        Profile::Claude => {
            if body.get("thinking").is_none() {
                if let Some(budget) = policy["budget_tokens"].as_u64() {
                    if body["max_tokens"]
                        .as_u64()
                        .is_some_and(|limit| limit <= budget)
                    {
                        return Err("Output token limit must exceed suffix thinking budget".into());
                    }
                    body["thinking"] = json!({"type":"enabled","budget_tokens":budget});
                }
            }
        }
        Profile::Gemini => {
            if body.pointer("/generationConfig/thinkingConfig").is_none() {
                if let Some(budget) = policy.get("budget_tokens") {
                    body["generationConfig"]["thinkingConfig"] = json!({"thinkingBudget":budget});
                }
            }
        }
        Profile::Unsupported => {
            return Err("Reasoning suffix is unsupported by this channel protocol".into())
        }
    }
    Ok(())
}
