use axum::http::HeaderMap;
use serde_json::{json, Value};

pub(crate) fn session_id(headers: &HeaderMap, payload: &Value, request_id: &str) -> String {
    for name in ["session_id", "session-id"] {
        if let Some(value) = headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .filter(|value| !value.is_empty())
        {
            return value.to_owned();
        }
    }
    payload
        .get("prompt_cache_key")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(|value| url::form_urlencoded::byte_serialize(value.as_bytes()).collect())
        .unwrap_or_else(|| request_id.to_owned())
}

pub(crate) fn search_history(payload: &mut Value) {
    let Some(root) = payload.as_object_mut() else {
        return;
    };
    let has_history = root
        .get("input")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items
                .iter()
                .any(|item| item.get("type").and_then(Value::as_str) == Some("web_search_call"))
        });
    if !has_history {
        return;
    }
    let tools = root.entry("tools").or_insert_with(|| json!([]));
    if tools.is_null() {
        *tools = json!([]);
    }
    let Some(tools) = tools.as_array_mut() else {
        return;
    };
    if tools.iter().any(|tool| {
        tool.get("type")
            .and_then(Value::as_str)
            .is_some_and(|kind| {
                kind == "web_search"
                    || kind == "web_search_preview"
                    || kind.starts_with("web_search_preview_")
            })
    }) {
        return;
    }
    let was_empty = tools.is_empty();
    tools.push(json!({"type":"web_search"}));
    if was_empty {
        root.insert("tool_choice".into(), json!("none"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_precedence_and_cache_only_requests_are_stable() {
        let mut headers = HeaderMap::new();
        let payload = json!({"prompt_cache_key":"fixed prefix/中文"});
        assert_eq!(
            session_id(&headers, &payload, "request-1"),
            session_id(&headers, &payload, "request-2")
        );
        assert!(!session_id(&headers, &payload, "request-1").contains('\n'));
        headers.insert("Session-Id", "hyphen".parse().unwrap());
        assert_eq!(session_id(&headers, &payload, "request"), "hyphen");
        headers.insert("session_id", "underscore".parse().unwrap());
        assert_eq!(session_id(&headers, &payload, "request"), "underscore");
        assert_eq!(
            session_id(&HeaderMap::new(), &json!({}), "request"),
            "request"
        );
    }

    #[test]
    fn search_history_repair_is_idempotent_and_does_not_enable_new_searches() {
        let mut no_history = json!({"input":[{"role":"user","content":"hi"}],"tool_choice":"auto"});
        let before = no_history.clone();
        search_history(&mut no_history);
        assert_eq!(no_history, before);
        for tools in [None, Some(Value::Null), Some(json!([]))] {
            let mut payload =
                json!({"input":[{"type":"web_search_call","id":"search-1"}],"tool_choice":"auto"});
            if let Some(tools) = tools {
                payload["tools"] = tools;
            }
            search_history(&mut payload);
            assert_eq!(payload["tool_choice"], "none");
            assert_eq!(payload["tools"], json!([{"type":"web_search"}]));
            let repaired = payload.clone();
            search_history(&mut payload);
            assert_eq!(payload, repaired);
            assert_eq!(payload["input"][0]["id"], "search-1");
        }
        for kind in [
            "web_search",
            "web_search_preview",
            "web_search_preview_2025_03_11",
        ] {
            let mut payload = json!({"input":[{"type":"web_search_call"}],"tools":[{"type":kind}],"tool_choice":"none"});
            let before = payload.clone();
            search_history(&mut payload);
            assert_eq!(payload, before);
        }
        let mut functions = json!({"input":[{"type":"web_search_call"}],"tools":[{"type":"function","name":"read"}],"tool_choice":{"type":"function","name":"read"}});
        search_history(&mut functions);
        assert_eq!(functions["tools"].as_array().unwrap().len(), 2);
        assert_eq!(functions["tool_choice"]["name"], "read");
    }
}
