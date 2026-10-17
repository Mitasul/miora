use crate::config::compiler::compile_snapshot_bytes;
use crate::config::compiler::expand_environment;
use crate::config::compiler::infer_engine;
use crate::config::compiler::scalar_string;
use futures_util::future::join_all;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde_json::{json, Value};
use std::collections::HashMap;
use url::Url;

pub(crate) async fn compile_snapshot_with_discovery(
    raw: &[u8],
    database_disabled: bool,
    client: &reqwest::Client,
    discovery_cache: &tokio::sync::Mutex<HashMap<String, Vec<String>>>,
) -> Result<Vec<u8>, String> {
    let yaml: serde_yaml::Value = serde_yaml::from_slice(raw)
        .map_err(|error| format!("decode uni-api YAML configuration: {error}"))?;
    let mut config = serde_json::to_value(yaml)
        .map_err(|error| format!("convert uni-api configuration: {error}"))?;
    expand_environment(&mut config);
    discover_missing_provider_models(&mut config, client, discovery_cache).await;
    let hydrated = serde_yaml::to_string(&config)
        .map_err(|error| format!("encode discovered runtime configuration: {error}"))?;
    compile_snapshot_bytes(hydrated.as_bytes(), database_disabled)
}

pub(crate) async fn discover_missing_provider_models(
    config: &mut Value,
    client: &reqwest::Client,
    discovery_cache: &tokio::sync::Mutex<HashMap<String, Vec<String>>>,
) {
    let Some(providers) = config.get_mut("providers").and_then(Value::as_array_mut) else {
        return;
    };
    let cached = discovery_cache.lock().await.clone();
    let mut pending = Vec::new();
    for (index, provider) in providers.iter_mut().enumerate() {
        if provider.get("enabled").and_then(Value::as_bool) == Some(false) {
            continue;
        }
        // Service-plugin discovery uses its authenticated registry endpoint, not
        // an inferred provider URL. Models are explicitly saved by admin API.
        if provider.get("channel_plugin").is_some()
            || provider
                .get("engine")
                .and_then(Value::as_str)
                .is_some_and(|e| e.starts_with("plugin:"))
        {
            continue;
        }
        if provider
            .get("model")
            .and_then(Value::as_array)
            .is_some_and(|models| !models.is_empty())
        {
            continue;
        }
        let cache_key = discovery_cache_key(provider);
        if let Some(models) = cached.get(&cache_key).filter(|models| !models.is_empty()) {
            provider
                .as_object_mut()
                .expect("provider configuration object")
                .insert(
                    "model".into(),
                    Value::Array(models.iter().cloned().map(Value::String).collect()),
                );
            continue;
        }
        pending.push({
            let provider = provider.clone();
            let client = client.clone();
            async move {
                (
                    index,
                    cache_key,
                    discover_provider_models(&client, &provider).await,
                )
            }
        });
    }
    let mut discovered = Vec::new();
    for (index, cache_key, result) in join_all(pending).await {
        match result {
            Ok(models) if !models.is_empty() => {
                discovered.push((cache_key, models.clone()));
                providers[index]
                    .as_object_mut()
                    .expect("provider configuration object")
                    .insert(
                        "model".into(),
                        Value::Array(models.into_iter().map(Value::String).collect()),
                    );
            }
            Ok(_) => {}
            Err(error) => eprintln!(
                "{}",
                json!({
                    "event_type": "rust_provider_model_discovery_error",
                    "provider": providers[index].get("provider").and_then(Value::as_str),
                    "error": error,
                })
            ),
        }
    }
    if !discovered.is_empty() {
        discovery_cache.lock().await.extend(discovered);
    }
}

pub(crate) fn discovery_cache_key(provider: &Value) -> String {
    provider.to_string()
}

pub(crate) async fn discover_provider_models(
    client: &reqwest::Client,
    provider: &Value,
) -> Result<Vec<String>, String> {
    let base_url = provider
        .get("base_url")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if base_url.is_empty() {
        return Err("provider base_url is required for model discovery".into());
    }
    let api_key = first_provider_key(provider.get("api"));
    let engine = provider
        .get("engine")
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| infer_engine(base_url));
    if engine == "azure" || base_url.contains("models.inference.ai.azure.com") {
        return Err(
            "Azure has no compatible model catalog here; enter deployment names manually".into(),
        );
    }
    if engine == "antigravity" {
        let mut url =
            Url::parse(base_url).map_err(|_| "Invalid Antigravity base URL".to_owned())?;
        let base_path = url.path().trim_end_matches('/');
        url.set_path(&format!("{base_path}/v1internal:fetchAvailableModels"));
        let mut req = client
            .post(url)
            .header("content-type", "application/json")
            .header("user-agent", "antigravity/1.23.2")
            .timeout(std::time::Duration::from_secs(20))
            .json(&json!({}));
        if !api_key.is_empty() {
            req = req.bearer_auth(&api_key);
        }
        if let Ok(resp) = req.send().await {
            if resp.status().is_success() {
                if let Ok(payload) = resp.json::<Value>().await {
                    let ids = discovered_model_ids(&payload);
                    if !ids.is_empty() {
                        return Ok(ids);
                    }
                }
            }
        }
    }
    let mut headers = HeaderMap::new();
    let url = if engine == "gemini" {
        gemini_models_url(base_url, &api_key)?
    } else {
        if !api_key.is_empty() {
            if engine == "claude" || engine == "vertex-claude" {
                if api_key.starts_with("sk-ant-oat") {
                    headers.insert(
                        AUTHORIZATION,
                        HeaderValue::from_str(&format!("Bearer {api_key}"))
                            .map_err(|_| "provider API key is not a valid header".to_owned())?,
                    );
                    headers.insert(
                        "anthropic-beta",
                        HeaderValue::from_static("oauth-2025-04-20"),
                    );
                } else {
                    headers.insert(
                        "x-api-key",
                        HeaderValue::from_str(&api_key)
                            .map_err(|_| "provider API key is not a valid header".to_owned())?,
                    );
                }
                headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
            } else {
                headers.insert(
                    AUTHORIZATION,
                    HeaderValue::from_str(&format!("Bearer {api_key}"))
                        .map_err(|_| "provider API key is not a valid header".to_owned())?,
                );
            }
        }
        provider_models_url(base_url)?
    };
    let response = client
        .get(url)
        .headers(headers)
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                "Model discovery timed out after 20 seconds".to_owned()
            } else {
                "Could not fetch the model catalog; check the provider address and network"
                    .to_owned()
            }
        })?;
    if !response.status().is_success() {
        return Err(format!(
            "Model discovery returned HTTP {}; check provider credentials and endpoint",
            response.status().as_u16()
        ));
    }
    let payload = response
        .json::<Value>()
        .await
        .map_err(|_| "Model discovery response is not valid JSON".to_owned())?;
    if !payload.is_array()
        && !payload.get("data").is_some_and(Value::is_array)
        && !payload
            .get("models")
            .is_some_and(|m| m.is_array() || m.is_object())
    {
        return Err("Model discovery response does not contain a models array".into());
    }
    Ok(discovered_model_ids(&payload))
}

fn gemini_models_url(base_url: &str, api_key: &str) -> Result<Url, String> {
    let mut url =
        Url::parse(base_url).map_err(|_| "Invalid Gemini model discovery URL".to_owned())?;
    let path = url.path().trim_end_matches('/');
    let path = if let Some((prefix, _)) = path.split_once("/v1beta") {
        format!("{prefix}/v1beta/models")
    } else if let Some((prefix, _)) = path.split_once("/v1") {
        format!("{prefix}/v1/models")
    } else {
        format!("{path}/v1beta/models")
    };
    url.set_path(&path);
    if !api_key.is_empty() {
        let pairs: Vec<_> = url
            .query_pairs()
            .filter(|(key, _)| key != "key")
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        url.set_query(None);
        url.query_pairs_mut()
            .extend_pairs(pairs)
            .append_pair("key", api_key);
    }
    Ok(url)
}

pub(crate) fn first_provider_key(value: Option<&Value>) -> String {
    provider_keys(value).into_iter().next().unwrap_or_default()
}

pub(crate) fn provider_keys(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::String(value)) => value
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect(),
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect(),
        Some(value) => {
            let single = scalar_string(value).trim().to_owned();
            if single.is_empty() {
                Vec::new()
            } else {
                vec![single]
            }
        }
        None => Vec::new(),
    }
}

pub(crate) fn provider_models_url(base_url: &str) -> Result<Url, String> {
    let mut url =
        Url::parse(base_url).map_err(|_| "Invalid provider model discovery URL".to_owned())?;
    let known = [
        "/systemone",
        "/chat/completions",
        "/responses/compact",
        "/responses",
        "/messages",
        "/embeddings",
        "/moderations",
    ];
    let mut path = url.path().trim_end_matches('/').to_owned();
    for suffix in known {
        if path.ends_with(suffix) {
            path.truncate(path.len() - suffix.len());
            break;
        }
    }
    if path.ends_with("/models") {
        return Ok(url);
    }
    if path.ends_with("/v1")
        || path.ends_with("/v2")
        || path.ends_with("/v3")
        || path.ends_with("/v4")
        || path.ends_with("/v1beta")
        || path.ends_with("/v1alpha")
        || path.ends_with("/openai")
    {
        path.push_str("/models");
    } else {
        path.push_str("/v1/models");
    }
    url.set_path(&path);
    Ok(url)
}

pub(crate) fn discovered_model_ids(payload: &Value) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    if let Some(map) = payload.get("models").and_then(Value::as_object) {
        for key in map.keys() {
            let value = key.trim_start_matches("models/").trim();
            if !value.is_empty() {
                seen.insert(value.to_owned());
            }
        }
    }
    let items = payload
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| payload.get("models").and_then(Value::as_array))
        .or_else(|| {
            payload
                .pointer("/data/models")
                .or_else(|| payload.pointer("/result/models"))
                .or_else(|| payload.get("result"))
                .and_then(Value::as_array)
        })
        .or_else(|| payload.as_array())
        .into_iter()
        .flatten();
    for item in items {
        let value = item
            .as_str()
            .or_else(|| item.get("id").and_then(Value::as_str))
            .or_else(|| item.get("slug").and_then(Value::as_str))
            .or_else(|| item.get("model").and_then(Value::as_str))
            .or_else(|| item.get("modelId").and_then(Value::as_str))
            .or_else(|| item.get("name").and_then(Value::as_str))
            .unwrap_or_default()
            .trim_start_matches("models/")
            .trim();
        if !value.is_empty() {
            seen.insert(value.to_owned());
        }
    }
    seen.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_discovery_parses_openai_gemini_and_plugin_lists() {
        assert_eq!(
            discovered_model_ids(
                &json!({"data":[{"id":"model-b"},{"id":"model-a"},{"id":"model-a"}]})
            ),
            vec!["model-a", "model-b"]
        );
        assert_eq!(
            discovered_model_ids(&json!({"models":[{"name":"models/gemini-example"}]})),
            vec!["gemini-example"]
        );
        assert_eq!(
            discovered_model_ids(&json!(["model-b",{"id":"model-a"}])),
            vec!["model-a", "model-b"]
        );
    }

    #[test]
    fn model_discovery_urls_keep_saved_query_without_duplicating_paths() {
        let url =
            provider_models_url("https://example.invalid/api/v1/chat/completions?credential=saved")
                .unwrap();
        assert_eq!(url.path(), "/api/v1/models");
        assert_eq!(url.query(), Some("credential=saved"));
        assert_eq!(
            provider_models_url("https://example.invalid/api/v1/models")
                .unwrap()
                .path(),
            "/api/v1/models"
        );
        let gemini = gemini_models_url(
            "https://example.invalid/v1beta/models/old:generateContent?key=old&tenant=one",
            "replacement",
        )
        .unwrap();
        assert_eq!(gemini.path(), "/v1beta/models");
        assert_eq!(
            gemini.query_pairs().filter(|(key, _)| key == "key").count(),
            1
        );
        assert!(gemini
            .query_pairs()
            .any(|(key, value)| key == "key" && value == "replacement"));
        assert!(gemini
            .query_pairs()
            .any(|(key, value)| key == "tenant" && value == "one"));
    }
}
