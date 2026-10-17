use super::*;
use crate::runtime::context::AppState;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
pub(crate) enum BalanceTime {
    Seconds(u64),
    Text(String),
}

#[derive(Default, Deserialize, Serialize)]
pub(crate) struct AccountBalance {
    pub plan: Option<String>,
    pub active: Option<bool>,
    pub available: Option<bool>,
    pub expires_at: Option<BalanceTime>,
    pub amount: Option<f64>,
    pub currency: Option<String>,
    pub unlimited: Option<bool>,
    #[serde(default)]
    pub windows: Vec<BalanceWindow>,
    #[serde(default)]
    pub buckets: Vec<BalanceAmount>,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct BalanceWindow {
    pub label: String,
    pub remaining_percent: Option<f64>,
    pub used_percent: Option<f64>,
    pub remaining: Option<f64>,
    pub unit: Option<String>,
    pub reset_at: Option<BalanceTime>,
    pub limit_seconds: Option<u64>,
    pub limit_reached: Option<bool>,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct BalanceAmount {
    pub label: String,
    pub amount: f64,
    pub unit: String,
}

impl AccountBalance {
    pub(crate) fn into_value(self, source: &str) -> Result<Value, String> {
        if self.plan.is_none()
            && self.amount.is_none()
            && self.unlimited != Some(true)
            && self.windows.is_empty()
            && self.buckets.is_empty()
        {
            return Err("Balance response contains no plan or quota information".into());
        }
        let mut value = serde_json::to_value(self).map_err(|_| "Invalid balance response")?;
        value["status"] = json!("ok");
        value["source"] = json!(source);
        Ok(value)
    }
}

pub(crate) async fn query_channel(
    state: &AppState,
    provider: &Provider,
    proxy: Option<&str>,
) -> Value {
    if provider.preferences.get("balance_query") == Some(&Value::Bool(false)) {
        return json!({"provider":provider.name.as_ref(),"status":"disabled","keys":[],"checked_at":now()});
    }
    let id = match state.plugins.balance_plugin_id(provider) {
        Ok(Some(id)) => id,
        Ok(None) => {
            return json!({"provider":provider.name.as_ref(),"status":"unsupported","keys":[],"checked_at":now()})
        }
        Err(_) => {
            return json!({"provider":provider.name.as_ref(),"status":"plugin_unavailable","keys":[],"checked_at":now()})
        }
    };
    let mut seen = HashSet::new();
    let keys: Vec<_> = provider
        .api_keys
        .iter()
        .enumerate()
        .filter(|(_, key)| seen.insert(key.as_str()))
        .map(|(position, key)| (position, key.clone()))
        .collect();
    let total = keys.len();
    let mut values: Vec<Value> =
        stream::iter(keys.into_iter().take(MAX_KEYS).map(|(position, key)| {
            let id = &id;
            async move {
                let mut value = state
                    .plugins
                    .balance_authenticated(&state.codex_oauth, provider, &key, proxy, id)
                    .await
                    .unwrap_or_else(|_| failure("plugin_error"));
                value["position"] = json!(position + 1);
                value["checked_at"] = json!(now());
                value
            }
            .boxed()
        }))
        .buffer_unordered(3)
        .collect()
        .await;
    values.sort_by_key(|value| value["position"].as_u64());
    json!({"provider":provider.name.as_ref(),"status":if total == 0 {"no_key"} else {"complete"},"keys":values,"key_count":total,"omitted_keys":total.saturating_sub(MAX_KEYS),"checked_at":now()})
}

#[cfg(test)]
mod tests;
