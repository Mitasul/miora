//! Compatibility imports; inference dispatch lives in the plugin registry.
#[cfg(test)]
pub(crate) use crate::plugins::builtin::{build_attempt, provider_headers};
pub(crate) use crate::plugins::builtin::{preview_channel_request, ALPHA_SEARCH_ENDPOINT};

pub(crate) fn caller_auth(
    provider: &crate::config::snapshot::Provider,
) -> Result<Option<(&'static str, String)>, String> {
    let crate::config::snapshot::ProviderCredential::CallerSupplied(key) = &provider.credential
    else {
        return Ok(None);
    };
    let (name, value) = match provider.engine.as_ref() {
        "claude" => ("x-api-key", key.to_string()),
        "azure" => ("api-key", key.to_string()),
        "gemini" => ("x-goog-api-key", key.to_string()),
        "azure-databricks" => {
            use base64::Engine;
            (
                "authorization",
                format!(
                    "Basic {}",
                    base64::engine::general_purpose::STANDARD.encode(format!("token:{key}"))
                ),
            )
        }
        "gpt" | "openrouter" => ("authorization", format!("Bearer {key}")),
        _ => return Err("Unsupported BYOK channel authentication".into()),
    };
    Ok(Some((name, value)))
}

pub(crate) fn apply_caller_auth(
    provider: &crate::config::snapshot::Provider,
    headers: &mut axum::http::HeaderMap,
) -> Result<(), String> {
    if let Some((name, value)) = caller_auth(provider)? {
        for name in ["authorization", "x-api-key", "api-key", "x-goog-api-key"] {
            headers.remove(name);
        }
        headers.insert(
            name,
            axum::http::HeaderValue::from_str(&value)
                .map_err(|_| "Invalid caller-supplied upstream credential")?,
        );
    }
    Ok(())
}
