use axum::http::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeaderInterceptor {
    pub id: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub order: i32,
    #[serde(default)]
    pub plugin_ids: Vec<String>,
    #[serde(default)]
    pub set_headers: BTreeMap<String, String>,
    #[serde(default)]
    pub remove_headers: Vec<String>,
}
// A deliberately narrow, independent extension point: never a credential hook.
fn permitted(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with("x-")
        && ![
            "key",
            "auth",
            "token",
            "secret",
            "cookie",
            "forwarded",
            "real-ip",
            "oaix",
            "uni-api",
            "miora",
            "request-id",
            "caller-request",
        ]
        .iter()
        .any(|s| name.contains(s))
}
pub(crate) fn validate_chain(chain: &mut [HeaderInterceptor]) -> Result<(), String> {
    if chain.len() > 64 {
        return Err("too many interceptors".into());
    }
    let mut ids = BTreeSet::new();
    for interceptor in chain.iter() {
        if !super::manifest::valid_id(&interceptor.id) || !ids.insert(&interceptor.id) {
            return Err("invalid or duplicate interceptor id".into());
        }
        if interceptor.set_headers.len() + interceptor.remove_headers.len() > 32
            || interceptor.plugin_ids.len() > 128
        {
            return Err("interceptor size limit exceeded".into());
        }
        for name in interceptor
            .set_headers
            .keys()
            .chain(interceptor.remove_headers.iter())
        {
            if !permitted(name) || name.parse::<HeaderName>().is_err() {
                return Err("interceptors may only modify non-sensitive custom x-* headers".into());
            }
        }
        for value in interceptor.set_headers.values() {
            if value.len() > 4096 || value.parse::<HeaderValue>().is_err() {
                return Err("invalid interceptor header value".into());
            }
        }
    }
    chain.sort_by(|a, b| (a.order, &a.id).cmp(&(b.order, &b.id)));
    Ok(())
}
pub(crate) fn apply(chain: &[HeaderInterceptor], id: &str, headers: &mut HeaderMap) {
    for item in chain
        .iter()
        .filter(|i| i.enabled && (i.plugin_ids.is_empty() || i.plugin_ids.iter().any(|p| p == id)))
    {
        for key in &item.remove_headers {
            headers.remove(key.as_str());
        }
        for (key, value) in &item.set_headers {
            if let (Ok(name), Ok(value)) = (key.parse::<HeaderName>(), value.parse::<HeaderValue>())
            {
                headers.insert(name, value);
            }
        }
    }
}
