use super::*;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
pub(crate) struct BalanceStore {
    path: Arc<PathBuf>,
    values: Arc<Mutex<Option<BTreeMap<String, Value>>>>,
}

impl BalanceStore {
    pub(crate) fn from_env() -> Self {
        Self::at(PathBuf::from(
            std::env::var("MIORA_BALANCE_STORE_PATH")
                .unwrap_or_else(|_| "data/channel-balances.json".into()),
        ))
    }

    fn at(path: PathBuf) -> Self {
        Self {
            path: Arc::new(path),
            values: Arc::new(Mutex::new(None)),
        }
    }

    async fn load(&self, slot: &mut Option<BTreeMap<String, Value>>) -> Result<(), String> {
        if slot.is_none() {
            *slot = Some(match tokio::fs::read(self.path.as_ref()).await {
                Ok(bytes) => serde_json::from_slice(&bytes)
                    .map_err(|_| "Cannot read saved channel balances: invalid JSON".to_owned())?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
                Err(error) => return Err(format!("Cannot read saved channel balances: {error}")),
            });
        }
        Ok(())
    }

    pub(crate) async fn read(&self) -> Result<BTreeMap<String, Value>, String> {
        let mut slot = self.values.lock().await;
        self.load(&mut slot).await?;
        Ok(slot.as_ref().unwrap().clone())
    }

    pub(crate) async fn save(
        &self,
        name: &str,
        revision: &str,
        mut result: Value,
        channels: &[String],
    ) -> Result<Value, String> {
        let mut slot = self.values.lock().await;
        self.load(&mut slot).await?;
        let mut values = slot.as_ref().unwrap().clone();
        if let Some(previous) = values
            .get(name)
            .filter(|old| old["config_revision"] == revision)
        {
            retain_success(previous, &mut result);
        }
        result["config_revision"] = json!(revision);
        values.retain(|name, _| channels.contains(name));
        values.insert(name.to_owned(), result.clone());
        let bytes = serde_json::to_vec(&values).map_err(|_| "Cannot encode channel balances")?;
        crate::config::source::atomic_write(self.path.as_ref(), &bytes).await?;
        *slot = Some(values);
        Ok(result)
    }
}

fn retain_success(previous: &Value, result: &mut Value) {
    if result["keys"].as_array().is_some_and(Vec::is_empty)
        && !matches!(
            result["status"].as_str(),
            Some("complete" | "no_key" | "disabled" | "unsupported")
        )
    {
        result["last_error"] = result["status"].clone();
        result["keys"] = previous["keys"].clone();
    }
    let Some(keys) = result["keys"].as_array_mut() else {
        return;
    };
    for key in keys {
        if key["status"] == "ok" {
            continue;
        }
        let matched = previous["keys"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|old| {
                old["status"] == "ok"
                    && if let Some(reference) = key["account_ref"].as_str() {
                        old["account_ref"] == reference
                    } else {
                        old["position"] == key["position"]
                    }
            });
        if let Some(old) = matched {
            let attempt = key.clone();
            *key = old.clone();
            key["last_error"] = attempt["status"].clone();
            key["last_attempt_at"] = attempt["checked_at"].clone();
        }
    }
}

#[cfg(test)]
mod tests;
