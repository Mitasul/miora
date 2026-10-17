use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

mod accounts;
pub(crate) use accounts::ResolvedOAuth;

const DEFAULT_AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
const DEFAULT_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const DEFAULT_REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
const DEFAULT_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

const CLAUDE_AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
const CLAUDE_TOKEN_URL: &str = "https://api.anthropic.com/v1/oauth/token";
const CLAUDE_REDIRECT_URI: &str = "https://platform.claude.com/oauth/code/callback";
const CLAUDE_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const CLAUDE_SCOPE: &str = "org:create_api_key user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";

const ANTIGRAVITY_AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const ANTIGRAVITY_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const ANTIGRAVITY_USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v2/userinfo";
const ANTIGRAVITY_REDIRECT_URI: &str = "http://localhost:8085/callback";
const ANTIGRAVITY_CLIENT_ID: &str =
    "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
const ANTIGRAVITY_CLIENT_SECRET: &str = "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";
const ANTIGRAVITY_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile https://www.googleapis.com/auth/cclog https://www.googleapis.com/auth/experimentsandconfigs";

#[derive(Clone, Default)]
pub struct AccountMetadata {
    pub email: Option<String>,
    pub account_id: Option<String>,
    pub plan_type: Option<String>,
    pub expires_at: Option<u64>,
}

#[derive(Clone)]
pub struct PendingOAuthSession {
    pub provider: String,
    pub code_verifier: String,
    pub redirect_uri: String,
    pub credential: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone)]
pub struct CodexOAuthManager {
    cache: Arc<Mutex<HashMap<String, CachedToken>>>,
    metadata: Arc<Mutex<HashMap<String, AccountMetadata>>>,
    sessions: Arc<Mutex<HashMap<String, PendingOAuthSession>>>,
    locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
    persisted: Arc<Mutex<Option<HashMap<String, SavedToken>>>>,
    store_path: Arc<PathBuf>,
    token_url: Arc<str>,
    client_id: Arc<str>,
    refresh_skew: Duration,
    #[cfg(test)]
    refresh_fixture: Option<Arc<accounts::RefreshFixture>>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize, PartialEq)]
struct CachedToken {
    access_token: String,
    refresh_token: String,
    expires_at: Option<u64>,
    email: Option<String>,
    plan_type: Option<String>,
    account_id: Option<String>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(untagged)]
enum SavedToken {
    Reference { account_ref: String },
    Legacy(String),
    Account(CachedToken),
}

impl From<&str> for SavedToken {
    fn from(value: &str) -> Self {
        Self::Legacy(value.to_owned())
    }
}

pub struct CodexAuth {
    pub bearer: String,
    pub account_id: Option<String>,
    pub expires_at: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OAuthProvider {
    Codex,
    Claude,
    Antigravity,
}

struct RefreshCredential {
    provider: OAuthProvider,
    account_id: Option<String>,
    refresh_token: String,
}

impl CodexOAuthManager {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(Mutex::new(HashMap::new())),
            metadata: Arc::new(Mutex::new(HashMap::new())),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            locks: Arc::new(Mutex::new(HashMap::new())),
            persisted: Arc::new(Mutex::new(None)),
            store_path: Arc::new(PathBuf::from(
                std::env::var("CODEX_REFRESH_TOKEN_STORE_PATH")
                    .unwrap_or_else(|_| "./data/codex_refresh_tokens.json".into()),
            )),
            token_url: std::env::var("CODEX_OAUTH_TOKEN_URL")
                .unwrap_or_else(|_| DEFAULT_TOKEN_URL.into())
                .into(),
            client_id: std::env::var("CODEX_OAUTH_CLIENT_ID")
                .unwrap_or_else(|_| DEFAULT_CLIENT_ID.into())
                .into(),
            refresh_skew: Duration::from_secs(30),
            #[cfg(test)]
            refresh_fixture: None,
        }
    }

    pub async fn start_oauth_login(
        &self,
        provider: Option<&str>,
        redirect_uri: Option<&str>,
    ) -> Value {
        use ring::rand::{SecureRandom, SystemRandom};
        let rng = SystemRandom::new();
        let mut verifier_bytes = [0u8; 32];
        let mut state_bytes = [0u8; 16];
        let _ = rng.fill(&mut verifier_bytes);
        let _ = rng.fill(&mut state_bytes);
        let code_verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);
        let code_challenge = URL_SAFE_NO_PAD
            .encode(ring::digest::digest(&ring::digest::SHA256, code_verifier.as_bytes()).as_ref());
        let state = URL_SAFE_NO_PAD.encode(state_bytes);
        let normalized = provider
            .map(|p| p.trim().to_ascii_lowercase())
            .unwrap_or_default();
        let provider_key = if normalized.contains("claude") || normalized.contains("anthropic") {
            "claude"
        } else if normalized.contains("antigravity") {
            "antigravity"
        } else {
            "codex"
        };
        let redirect = redirect_uri
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or(match provider_key {
                "claude" => CLAUDE_REDIRECT_URI,
                "antigravity" => ANTIGRAVITY_REDIRECT_URI,
                _ => DEFAULT_REDIRECT_URI,
            });
        let authorize_url = match provider_key {
            "claude" => url::Url::parse_with_params(
                CLAUDE_AUTHORIZE_URL,
                &[
                    ("code", "true"),
                    ("client_id", CLAUDE_CLIENT_ID),
                    ("response_type", "code"),
                    ("redirect_uri", redirect),
                    ("scope", CLAUDE_SCOPE),
                    ("code_challenge", &code_challenge),
                    ("code_challenge_method", "S256"),
                    ("state", &state),
                ],
            )
            .map(|u| u.to_string())
            .unwrap_or_else(|_| CLAUDE_AUTHORIZE_URL.to_owned()),
            "antigravity" => url::Url::parse_with_params(
                ANTIGRAVITY_AUTHORIZE_URL,
                &[
                    ("client_id", ANTIGRAVITY_CLIENT_ID),
                    ("redirect_uri", redirect),
                    ("response_type", "code"),
                    ("scope", ANTIGRAVITY_SCOPE),
                    ("state", &state),
                    ("code_challenge", &code_challenge),
                    ("code_challenge_method", "S256"),
                    ("access_type", "offline"),
                    ("prompt", "consent"),
                    ("include_granted_scopes", "true"),
                ],
            )
            .map(|u| u.to_string())
            .unwrap_or_else(|_| ANTIGRAVITY_AUTHORIZE_URL.to_owned()),
            _ => url::Url::parse_with_params(
                DEFAULT_AUTHORIZE_URL,
                &[
                    ("response_type", "code"),
                    ("client_id", self.client_id.as_ref()),
                    ("redirect_uri", redirect),
                    ("scope", "openid profile email offline_access"),
                    ("code_challenge", &code_challenge),
                    ("code_challenge_method", "S256"),
                    ("id_token_add_organizations", "true"),
                    ("codex_cli_simplified_flow", "true"),
                    ("state", &state),
                ],
            )
            .map(|u| u.to_string())
            .unwrap_or_else(|_| DEFAULT_AUTHORIZE_URL.to_owned()),
        };
        self.sessions.lock().await.insert(
            state.clone(),
            PendingOAuthSession {
                provider: provider_key.to_owned(),
                code_verifier: code_verifier.clone(),
                redirect_uri: redirect.to_owned(),
                credential: None,
                error: None,
            },
        );
        if provider_key == "codex" && redirect.starts_with("http://localhost:1455") {
            self.spawn_local_callback_listener(state.clone(), 1455);
        } else if provider_key == "antigravity" && redirect.starts_with("http://localhost:8085") {
            self.spawn_local_callback_listener(state.clone(), 8085);
        }
        serde_json::json!({
            "provider": provider_key,
            "authorize_url": authorize_url,
            "code_verifier": code_verifier,
            "redirect_uri": redirect,
            "state": state,
            "auto_callback": provider_key != "claude",
        })
    }

    fn spawn_local_callback_listener(&self, expected_state: String, port: u16) {
        let manager = self.clone();
        tokio::spawn(async move {
            let Ok(listener) = tokio::net::TcpListener::bind(("127.0.0.1", port)).await else {
                return;
            };
            let accept_res =
                tokio::time::timeout(Duration::from_secs(300), listener.accept()).await;
            let Ok(Ok((mut stream, _))) = accept_res else {
                return;
            };
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut buf = vec![0u8; 4096];
            let Ok(n) = stream.read(&mut buf).await else {
                return;
            };
            let req_text = String::from_utf8_lossy(&buf[..n]);
            let first_line = req_text.lines().next().unwrap_or("");
            let path_and_query = first_line.split_whitespace().nth(1).unwrap_or("/");
            let full_url = format!("http://localhost:{port}{path_and_query}");
            let parsed_state = url::Url::parse(&full_url)
                .ok()
                .and_then(|u| {
                    u.query_pairs()
                        .find(|(k, _)| k == "state")
                        .map(|(_, v)| v.into_owned())
                })
                .unwrap_or(expected_state.clone());
            let session = manager.sessions.lock().await.get(&parsed_state).cloned();
            let Some(session) = session else {
                return;
            };
            let exchange_res = manager
                .exchange_code_with_provider(
                    &session.provider,
                    &full_url,
                    &session.code_verifier,
                    Some(&parsed_state),
                    Some(&session.redirect_uri),
                    None,
                )
                .await;
            let html = match exchange_res {
                Ok(credential) => {
                    if let Some(entry) = manager.sessions.lock().await.get_mut(&parsed_state) {
                        entry.credential = Some(credential);
                    }
                    "<!doctype html><html lang=\"zh-CN\"><meta charset=\"utf-8\"><title>Miora 授权成功</title><body style=\"font-family:system-ui;padding:48px;text-align:center\"><h2>OAuth 账号授权成功</h2><p>已自动获取 Token 与账号邮箱，请返回 Miora 控制台（此窗口可关闭）。</p><script>setTimeout(()=>window.close(),1200)</script></body></html>".to_owned()
                }
                Err(err) => {
                    if let Some(entry) = manager.sessions.lock().await.get_mut(&parsed_state) {
                        entry.error = Some(err.clone());
                    }
                    format!("<!doctype html><html lang=\"zh-CN\"><meta charset=\"utf-8\"><title>授权失败</title><body style=\"font-family:system-ui;padding:48px;text-align:center\"><h2>授权换取 Token 失败</h2><p>{err}</p></body></html>")
                }
            };
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                html.len(),
                html
            );
            let _ = stream.write_all(resp.as_bytes()).await;
        });
    }

    pub async fn oauth_session_status(&self, state: &str) -> Value {
        let session = self.sessions.lock().await.get(state).cloned();
        let Some(session) = session else {
            return serde_json::json!({ "status": "not_found" });
        };
        if let Some(credential) = session.credential {
            let oauth = self.status(&credential).await;
            return serde_json::json!({
                "status": "completed",
                "credential": credential,
                "oauth": oauth,
            });
        }
        if let Some(error) = session.error {
            return serde_json::json!({
                "status": "failed",
                "error": error,
            });
        }
        serde_json::json!({ "status": "pending" })
    }

    pub async fn exchange_code_with_provider(
        &self,
        provider: &str,
        code_input: &str,
        code_verifier: &str,
        state_input: Option<&str>,
        redirect_uri: Option<&str>,
        proxy: Option<&str>,
    ) -> Result<String, String> {
        let lower = provider.to_ascii_lowercase();
        let is_claude = lower.contains("claude") || lower.contains("anthropic");
        let is_antigravity = lower.contains("antigravity");
        let raw_code = extract_oauth_code(code_input)?;
        let (code, code_state) = match raw_code.split_once('#') {
            Some((c, s)) => (c.trim().to_owned(), Some(s.trim().to_owned())),
            None => (raw_code, None),
        };
        let state_val = code_state.or_else(|| {
            state_input
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        });
        let session = if let Some(ref st) = state_val {
            self.sessions.lock().await.get(st).cloned()
        } else {
            None
        };
        let verifier = if !code_verifier.trim().is_empty() {
            code_verifier.trim().to_owned()
        } else {
            session
                .as_ref()
                .map(|s| s.code_verifier.clone())
                .unwrap_or_default()
        };
        let redirect = redirect_uri
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .or_else(|| session.as_ref().map(|s| s.redirect_uri.clone()))
            .unwrap_or_else(|| {
                if is_claude {
                    CLAUDE_REDIRECT_URI.to_owned()
                } else if is_antigravity {
                    ANTIGRAVITY_REDIRECT_URI.to_owned()
                } else {
                    DEFAULT_REDIRECT_URI.to_owned()
                }
            });
        let mut builder = reqwest::Client::builder().http1_only();
        if let Some(proxy) = proxy.filter(|value| !value.trim().is_empty()) {
            builder = builder.proxy(
                reqwest::Proxy::all(proxy)
                    .map_err(|error| format!("invalid OAuth proxy: {error}"))?,
            );
        }
        let client = builder
            .build()
            .map_err(|error| format!("build OAuth client: {error}"))?;
        let response = if is_claude {
            let mut body_json = serde_json::json!({
                "code": code,
                "grant_type": "authorization_code",
                "client_id": CLAUDE_CLIENT_ID,
                "redirect_uri": redirect,
                "code_verifier": verifier,
            });
            if let Some(st) = state_val.as_ref() {
                body_json["state"] = Value::String(st.clone());
            }
            client
                .post(CLAUDE_TOKEN_URL)
                .header("content-type", "application/json")
                .header("accept", "application/json")
                .timeout(Duration::from_secs(30))
                .json(&body_json)
                .send()
                .await
                .map_err(|error| format!("Claude OAuth code exchange failed: {error}"))?
        } else if is_antigravity {
            let secret = std::env::var("ANTIGRAVITY_OAUTH_CLIENT_SECRET")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| ANTIGRAVITY_CLIENT_SECRET.to_owned());
            let form_body = {
                let mut form = url::form_urlencoded::Serializer::new(String::new());
                form.append_pair("client_id", ANTIGRAVITY_CLIENT_ID)
                    .append_pair("client_secret", &secret)
                    .append_pair("grant_type", "authorization_code")
                    .append_pair("code", &code)
                    .append_pair("redirect_uri", &redirect);
                if !verifier.is_empty() {
                    form.append_pair("code_verifier", &verifier);
                }
                form.finish()
            };
            client
                .post(ANTIGRAVITY_TOKEN_URL)
                .header("content-type", "application/x-www-form-urlencoded")
                .header("accept", "application/json")
                .timeout(Duration::from_secs(30))
                .body(form_body)
                .send()
                .await
                .map_err(|error| format!("Antigravity OAuth code exchange failed: {error}"))?
        } else {
            let form_body = {
                let mut form = url::form_urlencoded::Serializer::new(String::new());
                form.append_pair("client_id", self.client_id.as_ref())
                    .append_pair("grant_type", "authorization_code")
                    .append_pair("code", &code)
                    .append_pair("redirect_uri", &redirect);
                if !verifier.is_empty() {
                    form.append_pair("code_verifier", &verifier);
                }
                form.finish()
            };
            client
                .post(self.token_url.as_ref())
                .header("content-type", "application/x-www-form-urlencoded")
                .header("accept", "application/json")
                .timeout(Duration::from_secs(30))
                .body(form_body)
                .send()
                .await
                .map_err(|error| format!("OAuth code exchange failed: {error}"))?
        };
        let status = response.status();
        let body = response
            .bytes()
            .await
            .map_err(|error| format!("read OAuth code exchange response: {error}"))?;
        if !status.is_success() {
            return Err(format!(
                "OAuth 换取 Token 失败 (HTTP {}): {}",
                status.as_u16(),
                String::from_utf8_lossy(&body)
            ));
        }
        let payload: Value = serde_json::from_slice(&body)
            .map_err(|error| format!("decode OAuth code exchange response: {error}"))?;
        let mut cached = parse_token_payload(&payload, None)?;
        if is_antigravity && cached.email.is_none() {
            if let Ok(info_resp) = client
                .get(ANTIGRAVITY_USERINFO_URL)
                .bearer_auth(&cached.access_token)
                .timeout(Duration::from_secs(10))
                .send()
                .await
            {
                if let Ok(info_json) = info_resp.json::<Value>().await {
                    cached.email = info_json
                        .get("email")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    if cached.account_id.is_none() {
                        cached.account_id = info_json
                            .get("id")
                            .and_then(Value::as_str)
                            .map(str::to_owned);
                    }
                }
            }
            if cached.plan_type.is_none() {
                cached.plan_type = Some("ANTIGRAVITY".to_owned());
            }
        }
        let account_id = cached.account_id.clone().unwrap_or_default();
        let raw_key = if !cached.refresh_token.is_empty() {
            let account = if is_claude {
                "oauth:claude"
            } else if is_antigravity {
                "oauth:antigravity"
            } else {
                &account_id
            };
            format!("{account},{}", cached.refresh_token)
        } else {
            cached.access_token.clone()
        };
        if !cached.refresh_token.is_empty() {
            self.publish_imported_token(&raw_key, cached.clone())
                .await?;
        } else {
            self.cache
                .lock()
                .await
                .insert(raw_key.clone(), cached.clone());
        }
        self.metadata.lock().await.insert(
            raw_key.clone(),
            AccountMetadata {
                email: cached.email.clone(),
                account_id: cached.account_id.clone(),
                plan_type: cached.plan_type.clone(),
                expires_at: cached.expires_at,
            },
        );
        if let Some(st) = state_val {
            if let Some(entry) = self.sessions.lock().await.get_mut(&st) {
                entry.credential = Some(raw_key.clone());
            }
        }
        Ok(raw_key)
    }

    pub async fn record_imported_metadata(
        &self,
        raw_key: &str,
        entry: &Value,
    ) -> Result<(), String> {
        let tokens = entry.get("tokens").unwrap_or(entry);
        let oauth_obj = entry.get("claudeAiOauth").unwrap_or(tokens);
        let id_jwt = tokens
            .get("id_token")
            .and_then(Value::as_str)
            .map(parse_jwt_metadata)
            .unwrap_or_default();
        let access_jwt = oauth_obj
            .get("access_token")
            .or_else(|| oauth_obj.get("accessToken"))
            .and_then(Value::as_str)
            .map(parse_jwt_metadata)
            .unwrap_or_default();
        let email = entry
            .get("email")
            .or_else(|| tokens.get("email"))
            .or_else(|| oauth_obj.get("email"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .or(id_jwt.email)
            .or(access_jwt.email);
        let plan_type = entry
            .get("plan_type")
            .or_else(|| tokens.get("plan_type"))
            .or_else(|| oauth_obj.get("subscriptionType"))
            .or_else(|| oauth_obj.get("plan_type"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .or(id_jwt.plan_type)
            .or(access_jwt.plan_type);
        let account_id = tokens
            .get("account_id")
            .or_else(|| entry.get("account_id"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .or(id_jwt.account_id)
            .or(access_jwt.account_id);
        let expires_at = oauth_obj
            .get("expires_at")
            .or_else(|| oauth_obj.get("expiresAt"))
            .and_then(Value::as_u64)
            .map(|v| if v > 10_000_000_000 { v / 1000 } else { v })
            .or(access_jwt.expires_at)
            .or(id_jwt.expires_at);
        if let Some(credential) = split_refresh_key(raw_key)? {
            let access_token = oauth_obj
                .get("access_token")
                .or_else(|| oauth_obj.get("accessToken"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            if access_token.is_empty() && credential.provider != OAuthProvider::Codex {
                self.persist_token_record(raw_key, SavedToken::Legacy(credential.refresh_token))
                    .await?;
            } else {
                let token = CachedToken {
                    access_token,
                    refresh_token: credential.refresh_token,
                    expires_at: Some(expires_at.unwrap_or_else(unix_seconds)),
                    email: email.clone(),
                    plan_type: plan_type.clone(),
                    account_id: account_id.clone(),
                };
                self.publish_imported_token(raw_key, token).await?;
            }
        }
        if email.is_some() || plan_type.is_some() || account_id.is_some() || expires_at.is_some() {
            self.metadata.lock().await.insert(
                raw_key.to_owned(),
                AccountMetadata {
                    email,
                    account_id,
                    plan_type,
                    expires_at,
                },
            );
        }
        Ok(())
    }

    pub async fn resolve(&self, raw_key: &str, proxy: Option<&str>) -> Result<CodexAuth, String> {
        self.resolve_inner(raw_key, proxy, false).await
    }

    pub(crate) async fn refresh_now(
        &self,
        raw_key: &str,
        proxy: Option<&str>,
    ) -> Result<CodexAuth, String> {
        self.resolve_inner(raw_key, proxy, true).await
    }

    pub(crate) async fn status(&self, raw_key: &str) -> Value {
        let stored_key = match self.stored_key(raw_key).await {
            Ok(key) => key,
            Err(error) => return serde_json::json!({"error":error}),
        };
        let saved = self
            .persisted
            .lock()
            .await
            .as_ref()
            .and_then(|tokens| tokens.get(&stored_key))
            .cloned();
        let cached = self.cache.lock().await;
        let meta_map = self.metadata.lock().await;
        let token = cached.get(&stored_key).or(match &saved {
            Some(SavedToken::Account(token)) => Some(token),
            _ => None,
        });
        let meta = meta_map.get(raw_key);
        let jwt_meta = if raw_key.starts_with("eyJ") {
            parse_jwt_metadata(raw_key)
        } else {
            AccountMetadata::default()
        };
        let credential = split_refresh_key(raw_key).ok().flatten();
        let refreshable =
            credential.is_some() || stored_key.starts_with(accounts::CODEX_ACCOUNT_PREFIX);
        let split_account = credential.and_then(|credential| credential.account_id);
        let account = token
            .and_then(|t| t.account_id.clone())
            .or_else(|| meta.and_then(|m| m.account_id.clone()))
            .or(split_account)
            .or(jwt_meta.account_id);
        let email = token
            .and_then(|t| t.email.clone())
            .or_else(|| meta.and_then(|m| m.email.clone()))
            .or(jwt_meta.email);
        let plan_type = token
            .and_then(|t| t.plan_type.clone())
            .or_else(|| meta.and_then(|m| m.plan_type.clone()))
            .or(jwt_meta.plan_type);
        let expires_at = token
            .and_then(|t| t.expires_at)
            .or_else(|| meta.and_then(|m| m.expires_at))
            .or(jwt_meta.expires_at);
        serde_json::json!({
            "account_ref": stored_key.starts_with(accounts::CODEX_ACCOUNT_PREFIX).then_some(&stored_key),
            "refreshable": refreshable,
            "account_id": account,
            "email": email,
            "plan_type": plan_type,
            "cached": token.is_some(),
            "expires_at": expires_at,
        })
    }

    async fn resolve_inner(
        &self,
        raw_key: &str,
        proxy: Option<&str>,
        force: bool,
    ) -> Result<CodexAuth, String> {
        let raw_key = raw_key.trim();
        let credential = split_refresh_key(raw_key)?;
        let core = raw_key.starts_with("core-oauth:")
            || credential
                .as_ref()
                .is_some_and(|credential| credential.provider == OAuthProvider::Codex);
        let stored_key;
        let credential = if core {
            stored_key = self.account_reference(raw_key, "codex").await?;
            Some(RefreshCredential {
                provider: OAuthProvider::Codex,
                account_id: None,
                refresh_token: String::new(),
            })
        } else {
            stored_key = raw_key.to_owned();
            credential
        };
        let raw = stored_key.as_str();
        let Some(RefreshCredential {
            provider,
            account_id,
            refresh_token: configured_refresh,
        }) = credential
        else {
            return Ok(CodexAuth {
                bearer: raw.to_owned(),
                account_id: None,
                expires_at: None,
            });
        };
        let lock = self.refresh_lock(raw).await;
        let _guard = lock.lock().await;
        if let Some(token) = self.cache.lock().await.get(raw).cloned() {
            if !force && token_is_valid(&token, self.refresh_skew) {
                return Ok(CodexAuth {
                    bearer: token.access_token,
                    expires_at: token.expires_at,
                    account_id: if provider == OAuthProvider::Codex {
                        account_id.or(token.account_id)
                    } else {
                        None
                    },
                });
            }
        }
        self.ensure_store_loaded().await?;
        let persisted = self
            .persisted
            .lock()
            .await
            .as_ref()
            .and_then(|tokens| tokens.get(raw))
            .cloned();
        if let Some(SavedToken::Account(token)) = &persisted {
            if !force && token_is_valid(token, self.refresh_skew) {
                self.cache
                    .lock()
                    .await
                    .insert(raw.to_owned(), token.clone());
                return Ok(CodexAuth {
                    bearer: token.access_token.clone(),
                    expires_at: token.expires_at,
                    account_id: if provider == OAuthProvider::Codex {
                        account_id.or_else(|| token.account_id.clone())
                    } else {
                        None
                    },
                });
            }
        }
        let refresh_token = self
            .cache
            .lock()
            .await
            .get(raw)
            .map(|token| token.refresh_token.clone())
            .or_else(|| {
                persisted.as_ref().and_then(|token| match token {
                    SavedToken::Legacy(refresh) => Some(refresh.clone()),
                    SavedToken::Account(token) => Some(token.refresh_token.clone()),
                    SavedToken::Reference { .. } => None,
                })
            })
            .unwrap_or(configured_refresh);
        let mut builder = reqwest::Client::builder().http1_only();
        if let Some(proxy) = proxy.filter(|value| !value.trim().is_empty()) {
            builder = builder.proxy(
                reqwest::Proxy::all(proxy)
                    .map_err(|error| format!("invalid Codex OAuth proxy: {error}"))?,
            );
        }
        let client = builder
            .build()
            .map_err(|error| format!("build Codex OAuth client: {error}"))?;
        let mut refreshed = self.refresh(&client, provider, &refresh_token).await?;
        if let Some(SavedToken::Account(previous)) = &persisted {
            refreshed.email = refreshed.email.or_else(|| previous.email.clone());
            refreshed.plan_type = refreshed.plan_type.or_else(|| previous.plan_type.clone());
            refreshed.account_id = refreshed.account_id.or_else(|| previous.account_id.clone());
        }
        self.persist_token_record(raw, SavedToken::Account(refreshed.clone()))
            .await?;
        self.cache
            .lock()
            .await
            .insert(raw.to_owned(), refreshed.clone());
        Ok(CodexAuth {
            bearer: refreshed.access_token,
            expires_at: refreshed.expires_at,
            account_id: if provider == OAuthProvider::Codex {
                account_id.or(refreshed.account_id)
            } else {
                None
            },
        })
    }

    pub async fn clear(&self, raw_key: &str) {
        if let Ok(key) = self.stored_key(raw_key).await {
            self.cache.lock().await.remove(&key);
        }
    }

    async fn refresh(
        &self,
        client: &reqwest::Client,
        provider: OAuthProvider,
        refresh_token: &str,
    ) -> Result<CachedToken, String> {
        #[cfg(test)]
        if let Some(fixture) = &self.refresh_fixture {
            fixture
                .calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            tokio::task::yield_now().await;
            return fixture.result.clone();
        }
        let response = self
            .refresh_request(client, provider, refresh_token)
            .send()
            .await
            .map_err(|error| format!("OAuth token refresh request failed: {error}"))?;
        let status = response.status();
        let body = response
            .bytes()
            .await
            .map_err(|error| format!("Read OAuth token refresh response failed: {error}"))?;
        if !status.is_success() {
            return Err(format!(
                "OAuth token refresh failed: status {}: {}",
                status.as_u16(),
                String::from_utf8_lossy(&body)
            ));
        }
        let payload: Value = serde_json::from_slice(&body)
            .map_err(|error| format!("Decode OAuth token refresh response failed: {error}"))?;
        parse_token_payload(&payload, Some(refresh_token))
    }

    fn refresh_request(
        &self,
        client: &reqwest::Client,
        provider: OAuthProvider,
        refresh_token: &str,
    ) -> reqwest::RequestBuilder {
        let request = match provider {
            OAuthProvider::Claude => client.post(CLAUDE_TOKEN_URL).json(&serde_json::json!({
                "client_id": CLAUDE_CLIENT_ID,
                "grant_type": "refresh_token",
                "refresh_token": refresh_token,
            })),
            OAuthProvider::Antigravity => {
                let secret = std::env::var("ANTIGRAVITY_OAUTH_CLIENT_SECRET")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| ANTIGRAVITY_CLIENT_SECRET.to_owned());
                let body = url::form_urlencoded::Serializer::new(String::new())
                    .append_pair("client_id", ANTIGRAVITY_CLIENT_ID)
                    .append_pair("client_secret", &secret)
                    .append_pair("grant_type", "refresh_token")
                    .append_pair("refresh_token", refresh_token)
                    .finish();
                client
                    .post(ANTIGRAVITY_TOKEN_URL)
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(body)
            }
            OAuthProvider::Codex => {
                let body = url::form_urlencoded::Serializer::new(String::new())
                    .append_pair("client_id", self.client_id.as_ref())
                    .append_pair("grant_type", "refresh_token")
                    .append_pair("refresh_token", refresh_token)
                    .append_pair("scope", "openid profile email")
                    .finish();
                client
                    .post(self.token_url.as_ref())
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(body)
            }
        };
        request
            .header("accept", "application/json")
            .timeout(Duration::from_secs(30))
    }

    #[cfg(test)]
    async fn persisted_refresh_token(&self, raw_key: &str) -> Option<String> {
        let stored_key = self.stored_key(raw_key).await.ok()?;
        self.persisted.lock().await.as_ref().and_then(|tokens| {
            tokens.get(&stored_key).and_then(|token| match token {
                SavedToken::Legacy(refresh) => Some(refresh.clone()),
                SavedToken::Account(token) => Some(token.refresh_token.clone()),
                SavedToken::Reference { .. } => None,
            })
        })
    }

    #[cfg(test)]
    async fn persist_refresh_token(
        &self,
        raw_key: &str,
        refresh_token: &str,
    ) -> Result<(), String> {
        self.persist_token_record(raw_key, SavedToken::Legacy(refresh_token.to_owned()))
            .await
    }

    async fn persist_token_record(&self, raw_key: &str, token: SavedToken) -> Result<(), String> {
        self.ensure_store_loaded().await?;
        let mut persisted = self.persisted.lock().await;
        let values = persisted.get_or_insert_with(HashMap::new);
        let raw_key = match values.get(raw_key) {
            Some(SavedToken::Reference { account_ref }) => account_ref.as_str(),
            _ => raw_key,
        };
        if values.get(raw_key).is_some_and(|value| value == &token) {
            return Ok(());
        }
        let mut updated = values.clone();
        updated.insert(raw_key.to_owned(), token);
        self.write_token_store(&updated).await?;
        *persisted = Some(updated);
        Ok(())
    }

    async fn write_token_store(&self, updated: &HashMap<String, SavedToken>) -> Result<(), String> {
        let encoded = serde_json::to_vec_pretty(&updated)
            .map_err(|error| format!("encode OAuth credential store: {error}"))?;
        if let Some(parent) = self.store_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| format!("create OAuth store directory: {error}"))?;
        }
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let temporary = self.store_path.with_extension(format!(
            "tmp.{}.{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        tokio::fs::write(&temporary, encoded)
            .await
            .map_err(|error| format!("write OAuth credential store: {error}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600))
                .await
                .map_err(|error| format!("restrict OAuth credential store: {error}"))?;
        }
        if let Err(error) = tokio::fs::rename(&temporary, self.store_path.as_ref()).await {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(format!("publish OAuth credential store: {error}"));
        }
        Ok(())
    }

    async fn ensure_store_loaded(&self) -> Result<(), String> {
        let mut persisted = self.persisted.lock().await;
        if persisted.is_some() {
            return Ok(());
        }
        let values = match tokio::fs::read(self.store_path.as_ref()).await {
            Ok(bytes) => serde_json::from_slice::<HashMap<String, SavedToken>>(&bytes)
                .map_err(|error| format!("read OAuth credential store: {error}"))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(error) => return Err(format!("read OAuth credential store: {error}")),
        };
        *persisted = Some(values);
        Ok(())
    }
}

fn parse_token_payload(
    payload: &Value,
    fallback_refresh: Option<&str>,
) -> Result<CachedToken, String> {
    let access_token = payload
        .get("access_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| "Codex token response returned empty access_token".to_owned())?
        .to_owned();
    let refreshed = payload
        .get("refresh_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .or(fallback_refresh)
        .unwrap_or_default()
        .to_owned();
    let id_meta = payload
        .get("id_token")
        .and_then(Value::as_str)
        .map(parse_jwt_metadata)
        .unwrap_or_default();
    let access_meta = parse_jwt_metadata(&access_token);
    let claude_email = payload
        .pointer("/account/email_address")
        .or_else(|| payload.get("email"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned);
    let claude_account = payload
        .pointer("/account/uuid")
        .or_else(|| payload.pointer("/organization/uuid"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned);
    let claude_plan = payload
        .pointer("/organization/name")
        .or_else(|| payload.get("plan_type"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned);
    let expires_at = payload
        .get("expires_in")
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
        })
        .filter(|seconds| *seconds > 0)
        .map(|seconds| unix_seconds().saturating_add(seconds))
        .or(access_meta.expires_at)
        .or(id_meta.expires_at);
    Ok(CachedToken {
        access_token,
        refresh_token: refreshed,
        expires_at,
        email: claude_email.or(id_meta.email).or(access_meta.email),
        plan_type: claude_plan.or(id_meta.plan_type).or(access_meta.plan_type),
        account_id: claude_account
            .or(id_meta.account_id)
            .or(access_meta.account_id),
    })
}

pub(crate) fn parse_jwt_metadata(token: &str) -> AccountMetadata {
    let mut parts = token.trim().split('.');
    let _header = parts.next();
    let Some(payload_b64) = parts.next() else {
        return AccountMetadata::default();
    };
    let Ok(decoded) = URL_SAFE_NO_PAD.decode(payload_b64.trim_end_matches('=')) else {
        return AccountMetadata::default();
    };
    let Ok(claims) = serde_json::from_slice::<Value>(&decoded) else {
        return AccountMetadata::default();
    };
    let auth_claim = claims.get("https://api.openai.com/auth");
    let profile_claim = claims.get("https://api.openai.com/profile");
    let email = claims
        .get("email")
        .or_else(|| profile_claim.and_then(|p| p.get("email")))
        .or_else(|| claims.get("upn"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned);
    let account_id = auth_claim
        .and_then(|a| a.get("chatgpt_account_id"))
        .or_else(|| claims.get("chatgpt_account_id"))
        .or_else(|| auth_claim.and_then(|a| a.get("user_id")))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned);
    let plan_type = auth_claim
        .and_then(|a| a.get("chatgpt_plan_type"))
        .or_else(|| claims.get("plan_type"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned);
    let expires_at = claims.get("exp").and_then(Value::as_u64);
    AccountMetadata {
        email,
        account_id,
        plan_type,
        expires_at,
    }
}

fn extract_oauth_code(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("请粘贴回调链接或授权码 (code)".into());
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        let url = url::Url::parse(trimmed).map_err(|_| "回调链接格式无效".to_owned())?;
        if let Some((_, error_desc)) = url
            .query_pairs()
            .find(|(k, _)| k == "error_description" || k == "error")
        {
            return Err(format!("OAuth 授权返回错误: {error_desc}"));
        }
        if let Some((_, code)) = url.query_pairs().find(|(k, _)| k == "code") {
            let code = code.trim().to_owned();
            if !code.is_empty() {
                return Ok(code);
            }
        }
        return Err("回调链接中未找到 code 参数".into());
    }
    Ok(trimmed.to_owned())
}

fn split_refresh_key(raw: &str) -> Result<Option<RefreshCredential>, String> {
    let Some((account_id, refresh_token)) = raw.split_once(',') else {
        return Ok(None);
    };
    let refresh_token = refresh_token.trim();
    if refresh_token.is_empty() {
        return Err("Invalid Codex API key format: expected 'account_id,refresh_token'".into());
    }
    let (provider, account_id) = match account_id.trim() {
        "oauth:claude" => (OAuthProvider::Claude, None),
        "oauth:antigravity" => (OAuthProvider::Antigravity, None),
        account => (
            OAuthProvider::Codex,
            (!account.is_empty()).then(|| account.to_owned()),
        ),
    };
    Ok(Some(RefreshCredential {
        provider,
        account_id,
        refresh_token: refresh_token.to_owned(),
    }))
}

fn token_is_valid(token: &CachedToken, skew: Duration) -> bool {
    token
        .expires_at
        .is_none_or(|expires_at| unix_seconds() < expires_at.saturating_sub(skew.as_secs()))
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store_dir(prefix: &str) -> PathBuf {
        static SEQUENCE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "{prefix}-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ))
    }

    #[tokio::test]
    async fn migration_compat_imported_accounts_and_rotated_tokens_survive_restart_without_network()
    {
        let directory = temp_store_dir("miora-migration-oauth");
        let store = Arc::new(directory.join("tokens.json"));
        let manager = CodexOAuthManager {
            store_path: store.clone(),
            token_url: "no-network://token".into(),
            ..CodexOAuthManager::new()
        };
        let expiry = unix_seconds() + 3600;
        for raw in [
            "account,refresh-original",
            "oauth:claude,refresh-original",
            "oauth:antigravity,refresh-original",
        ] {
            manager.record_imported_metadata(raw, &serde_json::json!({"tokens":{"access_token":"imported-access","refresh_token":"refresh-original","account_id":"metadata-account","expires_at":expiry * 1000},"email":"import@example.invalid","plan_type":"plan"})).await.unwrap();
        }
        let restarted = CodexOAuthManager {
            store_path: store.clone(),
            token_url: "no-network://token".into(),
            ..CodexOAuthManager::new()
        };
        for raw in [
            "account,refresh-original",
            "oauth:claude,refresh-original",
            "oauth:antigravity,refresh-original",
        ] {
            let auth = restarted.resolve(raw, None).await.unwrap();
            assert_eq!(auth.bearer, "imported-access");
            assert_eq!(
                auth.account_id.as_deref(),
                if raw.starts_with("account,") {
                    Some("account")
                } else {
                    None
                }
            );
            let status = restarted.status(raw).await;
            assert_eq!(status["email"], "import@example.invalid");
            assert_eq!(status["expires_at"], expiry);
        }
        let rotated = CachedToken {
            access_token: "rotated-access".into(),
            refresh_token: "refresh-rotated".into(),
            expires_at: Some(expiry),
            email: Some("import@example.invalid".into()),
            plan_type: Some("plan".into()),
            account_id: Some("metadata-account".into()),
        };
        restarted
            .persist_token_record("account,refresh-original", SavedToken::Account(rotated))
            .await
            .unwrap();
        let restarted = CodexOAuthManager {
            store_path: store.clone(),
            token_url: "no-network://token".into(),
            ..CodexOAuthManager::new()
        };
        assert_eq!(
            restarted
                .resolve("account,refresh-original", None)
                .await
                .unwrap()
                .bearer,
            "rotated-access"
        );
        assert_eq!(
            restarted
                .persisted_refresh_token("account,refresh-original")
                .await
                .as_deref(),
            Some("refresh-rotated")
        );
        tokio::fs::remove_file(store.as_ref()).await.unwrap();
        tokio::fs::remove_dir(directory).await.unwrap();
    }

    #[tokio::test]
    async fn migration_compat_import_without_expiry_is_not_permanent_and_bad_store_is_not_overwritten(
    ) {
        let directory = temp_store_dir("miora-migration-expiry");
        let store = Arc::new(directory.join("tokens.json"));
        let manager = CodexOAuthManager {
            store_path: store.clone(),
            ..CodexOAuthManager::new()
        };
        manager
            .record_imported_metadata(
                "account,refresh",
                &serde_json::json!({"access_token":"temporary"}),
            )
            .await
            .unwrap();
        let reference = manager
            .account_reference("account,refresh", "codex")
            .await
            .unwrap();
        assert!(!token_is_valid(
            &manager.cache.lock().await[&reference],
            Duration::ZERO
        ));
        tokio::fs::write(store.as_ref(), b"not-json").await.unwrap();
        let restarted = CodexOAuthManager {
            store_path: store.clone(),
            ..CodexOAuthManager::new()
        };
        assert!(restarted
            .persist_refresh_token("other,refresh", "new")
            .await
            .is_err());
        assert_eq!(tokio::fs::read(store.as_ref()).await.unwrap(), b"not-json");
        tokio::fs::remove_file(store.as_ref()).await.unwrap();
        tokio::fs::remove_dir(directory).await.unwrap();
    }

    #[test]
    fn parses_static_and_refresh_credentials() {
        assert!(split_refresh_key("token").unwrap().is_none());
        let credential = split_refresh_key("acct, refresh").unwrap().unwrap();
        assert_eq!(credential.provider, OAuthProvider::Codex);
        assert_eq!(credential.account_id.as_deref(), Some("acct"));
        assert_eq!(credential.refresh_token, "refresh");
        assert!(split_refresh_key("acct,").is_err());
    }

    #[test]
    fn parses_provider_refresh_credentials_without_codex_account_headers() {
        for (raw_key, provider) in [
            ("oauth:claude,refresh", OAuthProvider::Claude),
            ("oauth:antigravity,refresh", OAuthProvider::Antigravity),
        ] {
            let credential = split_refresh_key(raw_key).unwrap().unwrap();
            assert_eq!(credential.provider, provider);
            assert!(credential.account_id.is_none());
            assert_eq!(credential.refresh_token, "refresh");
        }
    }

    #[test]
    fn refresh_requests_use_provider_endpoints_and_body_formats() {
        let manager = CodexOAuthManager::new();
        let client = reqwest::Client::new();
        let claude = manager
            .refresh_request(&client, OAuthProvider::Claude, "claude-refresh")
            .build()
            .unwrap();
        assert_eq!(claude.url().as_str(), CLAUDE_TOKEN_URL);
        let body: Value =
            serde_json::from_slice(claude.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(body["client_id"], CLAUDE_CLIENT_ID);
        assert_eq!(body["grant_type"], "refresh_token");
        assert_eq!(body["refresh_token"], "claude-refresh");
        for (provider, endpoint, client_id) in [
            (
                OAuthProvider::Codex,
                manager.token_url.as_ref(),
                manager.client_id.as_ref(),
            ),
            (
                OAuthProvider::Antigravity,
                ANTIGRAVITY_TOKEN_URL,
                ANTIGRAVITY_CLIENT_ID,
            ),
        ] {
            let request = manager
                .refresh_request(&client, provider, "refresh+token")
                .build()
                .unwrap();
            assert_eq!(request.url().as_str(), endpoint);
            let form: HashMap<_, _> =
                url::form_urlencoded::parse(request.body().unwrap().as_bytes().unwrap())
                    .into_owned()
                    .collect();
            assert_eq!(form["client_id"], client_id);
            assert_eq!(form["grant_type"], "refresh_token");
            assert_eq!(form["refresh_token"], "refresh+token");
            if provider == OAuthProvider::Antigravity {
                assert!(form.contains_key("client_secret"));
            }
        }
    }

    #[tokio::test]
    async fn provider_credentials_resolve_cached_access_tokens() {
        let manager = CodexOAuthManager::new();
        for raw_key in ["oauth:claude,refresh", "oauth:antigravity,refresh"] {
            let cached = parse_token_payload(
                &serde_json::json!({
                    "access_token": "cached-access",
                    "refresh_token": "refresh",
                    "expires_in": 3600
                }),
                None,
            )
            .unwrap();
            manager
                .cache
                .lock()
                .await
                .insert(raw_key.to_owned(), cached);
            let auth = manager.resolve(raw_key, None).await.unwrap();
            assert_eq!(auth.bearer, "cached-access");
            assert!(auth.account_id.is_none());
            assert_eq!(manager.status(raw_key).await["refreshable"], true);
        }
    }

    #[tokio::test]
    async fn provider_refresh_token_rotations_survive_manager_restart() {
        let directory = temp_store_dir("miora-provider-oauth");
        let store = Arc::new(directory.join("tokens.json"));
        let manager = CodexOAuthManager {
            store_path: store.clone(),
            ..CodexOAuthManager::new()
        };
        for raw_key in ["oauth:claude,original", "oauth:antigravity,original"] {
            manager
                .persist_refresh_token(raw_key, "original")
                .await
                .unwrap();
            manager
                .persist_refresh_token(raw_key, "rotated")
                .await
                .unwrap();
        }
        let restarted = CodexOAuthManager {
            store_path: store.clone(),
            ..CodexOAuthManager::new()
        };
        for raw_key in ["oauth:claude,original", "oauth:antigravity,original"] {
            assert_eq!(
                restarted.persisted_refresh_token(raw_key).await.as_deref(),
                Some("rotated")
            );
        }
        tokio::fs::remove_file(store.as_ref()).await.unwrap();
        tokio::fs::remove_dir(directory).await.unwrap();
    }

    #[tokio::test]
    async fn gateway_features_failed_token_publication_is_not_cached_as_saved() {
        let directory = temp_store_dir("miora-oauth");
        let store = directory.join("tokens.json");
        tokio::fs::create_dir_all(&store).await.unwrap();
        let manager = CodexOAuthManager {
            store_path: Arc::new(store.clone()),
            ..CodexOAuthManager::new()
        };
        *manager.persisted.lock().await =
            Some(HashMap::from([("account,original".into(), "old".into())]));
        assert!(manager
            .persist_refresh_token("account,original", "new")
            .await
            .is_err());
        assert_eq!(
            manager
                .persisted_refresh_token("account,original")
                .await
                .as_deref(),
            Some("old")
        );
        tokio::fs::remove_dir(&store).await.unwrap();
        manager
            .persist_refresh_token("account,original", "new")
            .await
            .unwrap();
        assert_eq!(
            manager
                .persisted_refresh_token("account,original")
                .await
                .as_deref(),
            Some("new")
        );
        let saved: Value = serde_json::from_slice(&tokio::fs::read(&store).await.unwrap()).unwrap();
        assert_eq!(saved["account,original"], "new");
        tokio::fs::remove_file(&store).await.unwrap();
        tokio::fs::remove_dir(directory).await.unwrap();
    }
}
