use super::*;
use serde::Serialize;

pub(crate) const CODEX_ACCOUNT_PREFIX: &str = "core-oauth:codex:";

#[derive(Clone, Serialize)]
pub(crate) struct ResolvedOAuth {
    pub provider: String,
    pub account_ref: String,
    pub access_token: String,
    pub account_id: String,
    pub expires_at: Option<u64>,
}

impl CodexOAuthManager {
    pub(super) async fn refresh_lock(&self, key: &str) -> Arc<Mutex<()>> {
        self.locks
            .lock()
            .await
            .entry(key.to_owned())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    pub(super) async fn publish_imported_token(
        &self,
        raw: &str,
        mut token: CachedToken,
    ) -> Result<(), String> {
        let credential = split_refresh_key(raw)?;
        let key = if credential
            .as_ref()
            .is_some_and(|credential| credential.provider == OAuthProvider::Codex)
        {
            token.account_id = credential
                .and_then(|credential| credential.account_id)
                .or(token.account_id);
            self.account_reference(raw, "codex").await?
        } else {
            self.stored_key(raw).await?
        };
        let lock = self.refresh_lock(&key).await;
        let _guard = lock.lock().await;
        self.persist_token_record(&key, SavedToken::Account(token.clone()))
            .await?;
        self.cache.lock().await.insert(key, token);
        Ok(())
    }

    pub(crate) async fn account_reference(
        &self,
        raw: &str,
        provider: &str,
    ) -> Result<String, String> {
        if provider != "codex" {
            return Err("Unsupported core OAuth provider".into());
        }
        let raw = raw.trim();
        self.ensure_store_loaded().await?;
        let mut persisted = self.persisted.lock().await;
        let values = persisted.as_ref().unwrap();
        if let Some(SavedToken::Reference { account_ref }) = values.get(raw) {
            return Ok(account_ref.clone());
        }
        if raw.starts_with("core-oauth:") {
            if raw.starts_with(CODEX_ACCOUNT_PREFIX)
                && matches!(values.get(raw), Some(SavedToken::Account(_)))
            {
                return Ok(raw.to_owned());
            }
            return Err("Core OAuth account is missing or belongs to another provider".into());
        }
        let credential = split_refresh_key(raw)?
            .filter(|credential| credential.provider == OAuthProvider::Codex)
            .ok_or("Select or import a core Codex OAuth account with a refresh token")?;
        let mut cached = self.cache.lock().await;
        let mut token = cached
            .get(raw)
            .cloned()
            .or_else(|| match values.get(raw) {
                Some(SavedToken::Account(token)) => Some(token.clone()),
                _ => None,
            })
            .unwrap_or_else(|| CachedToken {
                access_token: String::new(),
                refresh_token: match values.get(raw) {
                    Some(SavedToken::Legacy(refresh)) => refresh.clone(),
                    _ => credential.refresh_token,
                },
                account_id: credential.account_id.clone(),
                expires_at: Some(0),
                email: None,
                plan_type: None,
            });
        token.account_id = credential.account_id.or(token.account_id);
        let existing = values.iter().find_map(|(reference, entry)| {
            if !reference.starts_with(CODEX_ACCOUNT_PREFIX) {
                return None;
            }
            let SavedToken::Account(previous) = entry else {
                return None;
            };
            (token.account_id.is_some() && previous.account_id == token.account_id)
                .then(|| reference.clone())
        });
        let reference = if let Some(reference) = existing {
            reference
        } else {
            use ring::rand::{SecureRandom, SystemRandom};
            let mut bytes = [0u8; 16];
            SystemRandom::new()
                .fill(&mut bytes)
                .map_err(|_| "Generate OAuth account reference")?;
            format!("{CODEX_ACCOUNT_PREFIX}{}", URL_SAFE_NO_PAD.encode(bytes))
        };
        let mut updated = values.clone();
        if !updated.contains_key(&reference) {
            updated.insert(reference.clone(), SavedToken::Account(token.clone()));
        }
        updated.insert(
            raw.to_owned(),
            SavedToken::Reference {
                account_ref: reference.clone(),
            },
        );
        self.write_token_store(&updated).await?;
        *persisted = Some(updated);
        cached.remove(raw);
        Ok(reference)
    }

    pub(crate) async fn resolve_core(
        &self,
        raw: &str,
        provider: &str,
        proxy: Option<&str>,
    ) -> Result<ResolvedOAuth, String> {
        let account_ref = self.account_reference(raw, provider).await?;
        let auth = self.resolve(&account_ref, proxy).await?;
        Ok(ResolvedOAuth {
            provider: provider.to_owned(),
            account_ref,
            access_token: auth.bearer,
            account_id: auth.account_id.ok_or("Codex OAuth account has no account_id; import its account ID before using this plugin")?,
            expires_at: auth.expires_at,
        })
    }

    pub(crate) async fn core_accounts(&self) -> Result<Vec<Value>, String> {
        self.ensure_store_loaded().await?;
        let keys: Vec<String> = self
            .persisted
            .lock()
            .await
            .as_ref()
            .unwrap()
            .keys()
            .filter(|key| {
                key.starts_with(CODEX_ACCOUNT_PREFIX)
                    || split_refresh_key(key)
                        .ok()
                        .flatten()
                        .is_some_and(|credential| credential.provider == OAuthProvider::Codex)
            })
            .cloned()
            .collect();
        let mut references = std::collections::BTreeSet::new();
        for key in keys {
            references.insert(self.account_reference(&key, "codex").await?);
        }
        let mut accounts = Vec::new();
        for reference in references {
            let mut status = self.status(&reference).await;
            status["account_ref"] = serde_json::json!(reference);
            status["provider"] = serde_json::json!("codex");
            accounts.push(status);
        }
        Ok(accounts)
    }

    pub(super) async fn stored_key(&self, raw: &str) -> Result<String, String> {
        self.ensure_store_loaded().await?;
        let persisted = self.persisted.lock().await;
        Ok(match persisted.as_ref().unwrap().get(raw) {
            Some(SavedToken::Reference { account_ref }) => account_ref.clone(),
            _ => raw.to_owned(),
        })
    }
}

#[cfg(test)]
pub(super) struct RefreshFixture {
    pub calls: std::sync::atomic::AtomicUsize,
    pub result: Result<CachedToken, String>,
}

#[cfg(test)]
mod tests;
