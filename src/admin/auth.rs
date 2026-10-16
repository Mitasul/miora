//! Separate password-authenticated administrator security domain.
use axum::extract::{ConnectInfo, Request};
use axum::http::{HeaderMap, StatusCode};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use ring::{
    digest, pbkdf2,
    rand::{SecureRandom, SystemRandom},
};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::Semaphore;

pub(super) const COOKIE: &str = "miora_admin_session";
const ITERATIONS: u32 = 600_000;
const MAX_SESSIONS: usize = 4096;
const MAX_CLIENTS: usize = 4096;
const LOGIN_WINDOW: Duration = Duration::from_secs(60);
const LOGIN_ATTEMPTS: u32 = 8;

#[derive(Clone)]
struct PasswordHash {
    rounds: NonZeroU32,
    salt: Vec<u8>,
    hash: Vec<u8>,
}

impl PasswordHash {
    fn from_password(password: &str) -> Result<Self, String> {
        if !(12..=1024).contains(&password.len()) {
            return Err("MIORA_ADMIN_PASSWORD must contain 12-1024 UTF-8 bytes".into());
        }
        let mut salt = vec![0; 32];
        SystemRandom::new()
            .fill(&mut salt)
            .map_err(|_| "Admin secure randomness unavailable")?;
        let rounds = NonZeroU32::new(ITERATIONS).unwrap();
        let mut hash = vec![0; 32];
        pbkdf2::derive(
            pbkdf2::PBKDF2_HMAC_SHA256,
            rounds,
            &salt,
            password.as_bytes(),
            &mut hash,
        );
        Ok(Self { rounds, salt, hash })
    }

    fn parse(encoded: &str) -> Result<Self, String> {
        let parts: Vec<_> = encoded.split('$').collect();
        let invalid = || {
            "MIORA_ADMIN_PASSWORD_HASH must be pbkdf2-sha256$iterations$hex-salt$hex-hash (100000-2000000 iterations, 16-64 byte salt, 32 byte hash)".to_owned()
        };
        if parts.len() != 4 || parts[0] != "pbkdf2-sha256" {
            return Err(invalid());
        }
        let rounds = parts[1]
            .parse::<u32>()
            .ok()
            .filter(|n| (100_000..=2_000_000).contains(n))
            .and_then(NonZeroU32::new)
            .ok_or_else(invalid)?;
        let salt = hex::decode(parts[2]).map_err(|_| invalid())?;
        let hash = hex::decode(parts[3]).map_err(|_| invalid())?;
        if !(16..=64).contains(&salt.len()) || hash.len() != 32 {
            return Err(invalid());
        }
        Ok(Self { rounds, salt, hash })
    }

    fn verify(&self, password: &str) -> bool {
        pbkdf2::verify(
            pbkdf2::PBKDF2_HMAC_SHA256,
            self.rounds,
            &self.salt,
            password.as_bytes(),
            &self.hash,
        )
        .is_ok()
    }
}

#[derive(Clone)]
pub(super) struct Session {
    pub csrf: String,
    pub expires_at: u64,
    expires: Instant,
}

struct AttemptWindow {
    started: Instant,
    attempts: u32,
}

struct Inner {
    username: String,
    password: Option<PasswordHash>,
    secure_cookie: bool,
    origin: Option<String>,
    trusted_proxies: HashSet<IpAddr>,
    ttl: Duration,
    sessions: Mutex<HashMap<[u8; 32], Session>>,
    attempts: Mutex<HashMap<String, AttemptWindow>>,
    hashing: Arc<Semaphore>,
    audit: Mutex<VecDeque<Value>>,
    pub config_write: tokio::sync::Mutex<()>,
}

#[derive(Clone)]
pub(crate) struct AdminState {
    inner: Arc<Inner>,
}

impl AdminState {
    /// Missing credentials disable login; malformed or weak configuration fails startup.
    pub(crate) fn from_env() -> Result<Self, String> {
        let username = std::env::var("MIORA_ADMIN_USERNAME").unwrap_or_else(|_| "admin".into());
        if username.is_empty() || username.len() > 128 || username.chars().any(char::is_control) {
            return Err("MIORA_ADMIN_USERNAME must contain 1-128 non-control bytes".into());
        }
        let password = match std::env::var("MIORA_ADMIN_PASSWORD_HASH") {
            Ok(encoded) => Some(PasswordHash::parse(&encoded)?),
            Err(_) => match std::env::var("MIORA_ADMIN_PASSWORD") {
                Ok(value) => Some(PasswordHash::from_password(&value)?),
                Err(_) => None,
            },
        };
        let secure_cookie = match std::env::var("MIORA_ADMIN_SECURE_COOKIE").as_deref() {
            Ok("true" | "1") | Err(_) => true,
            Ok("false" | "0") => false,
            _ => return Err("MIORA_ADMIN_SECURE_COOKIE must be true or false".into()),
        };
        let ttl = match std::env::var("MIORA_ADMIN_SESSION_TTL_SECONDS") {
            Ok(value) => value
                .parse::<u64>()
                .ok()
                .filter(|v| (300..=2_592_000).contains(v))
                .ok_or("MIORA_ADMIN_SESSION_TTL_SECONDS must be 300-2592000")?,
            Err(_) => 259_200,
        };
        let origin = std::env::var("MIORA_ADMIN_ORIGIN").ok().map(|value| {
            let url = url::Url::parse(&value).map_err(|_| "MIORA_ADMIN_ORIGIN must be an HTTP(S) origin")?;
            if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() || url.path() != "/" {
                return Err("MIORA_ADMIN_ORIGIN must be an HTTP(S) origin without credentials, path or query");
            }
            if (url.scheme() == "https") != secure_cookie {
                return Err("MIORA_ADMIN_ORIGIN scheme must match MIORA_ADMIN_SECURE_COOKIE");
            }
            Ok(url.origin().ascii_serialization())
        }).transpose().map_err(str::to_owned)?;
        let mut state = Self::build(
            username,
            password,
            secure_cookie,
            origin,
            Duration::from_secs(ttl),
        );
        let trusted = std::env::var("MIORA_ADMIN_TRUSTED_PROXIES").unwrap_or_default();
        let mut proxies = HashSet::new();
        for address in trusted.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            proxies.insert(address.parse::<IpAddr>().map_err(|_| {
                "MIORA_ADMIN_TRUSTED_PROXIES must be a comma-separated list of exact IP addresses"
            })?);
        }
        Arc::get_mut(&mut state.inner).unwrap().trusted_proxies = proxies;
        Ok(state)
    }

    fn build(
        username: String,
        password: Option<PasswordHash>,
        secure_cookie: bool,
        origin: Option<String>,
        ttl: Duration,
    ) -> Self {
        let mut sessions_map = load_persisted_sessions();
        let now_unix = unix_seconds();
        sessions_map.retain(|_, s| s.expires_at > now_unix);
        Self {
            inner: Arc::new(Inner {
                username,
                password,
                secure_cookie,
                origin,
                ttl,
                trusted_proxies: HashSet::new(),
                sessions: Mutex::new(sessions_map),
                attempts: Mutex::new(HashMap::new()),
                hashing: Arc::new(Semaphore::new(4)),
                audit: Mutex::new(VecDeque::new()),
                config_write: tokio::sync::Mutex::new(()),
            }),
        }
    }

    pub(crate) fn configured(&self) -> bool {
        self.inner.password.is_some()
    }

    pub(crate) fn is_authenticated(&self, headers: &HeaderMap) -> bool {
        self.session(headers).is_some()
    }

    pub(super) fn config_lock(&self) -> &tokio::sync::Mutex<()> {
        &self.inner.config_write
    }

    pub(super) fn check_origin(&self, headers: &HeaderMap) -> bool {
        if headers
            .get("sec-fetch-site")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v != "same-origin" && v != "none")
        {
            return false;
        }
        let Some(origin) = headers.get("origin") else {
            return true;
        };
        if headers.get_all("origin").iter().count() != 1 {
            return false;
        }
        let Ok(origin) = origin.to_str() else {
            return false;
        };
        let Ok(parsed) = url::Url::parse(origin) else {
            return false;
        };
        let expected = if let Some(configured) = &self.inner.origin {
            configured.clone()
        } else {
            let Some(host) = headers.get("host").and_then(|v| v.to_str().ok()) else {
                return false;
            };
            let scheme = if self.inner.secure_cookie || parsed.scheme() == "https" {
                "https"
            } else {
                "http"
            };
            format!("{scheme}://{host}")
        };
        let Ok(expected) = url::Url::parse(&expected) else {
            return false;
        };
        parsed.username().is_empty()
            && parsed.password().is_none()
            && parsed.path() == "/"
            && parsed.query().is_none()
            && parsed.fragment().is_none()
            && parsed.origin() == expected.origin()
    }

    pub(super) fn client_id(&self, request: &Request) -> String {
        let Some(peer) = request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|peer| peer.0.ip())
        else {
            return "unknown-peer".into();
        };
        forwarded_client(peer, request.headers(), &self.inner.trusted_proxies).to_string()
    }

    pub(super) fn login_allowed(&self, client: &str) -> bool {
        let now = Instant::now();
        let Ok(mut attempts) = self.inner.attempts.lock() else {
            return false;
        };
        attempts.retain(|_, window| now.duration_since(window.started) < LOGIN_WINDOW);
        if !attempts.contains_key(client) && attempts.len() >= MAX_CLIENTS {
            return false;
        }
        let window = attempts.entry(client.to_owned()).or_insert(AttemptWindow {
            started: now,
            attempts: 0,
        });
        if window.attempts >= LOGIN_ATTEMPTS {
            return false;
        }
        window.attempts += 1;
        true
    }

    pub(super) async fn verify(
        &self,
        username: String,
        password: String,
    ) -> Result<bool, StatusCode> {
        let Some(hash) = self.inner.password.clone() else {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        };
        let permit = self
            .inner
            .hashing
            .clone()
            .try_acquire_owned()
            .map_err(|_| StatusCode::TOO_MANY_REQUESTS)?;
        let expected = digest::digest(&digest::SHA256, self.inner.username.as_bytes());
        let actual = digest::digest(&digest::SHA256, username.as_bytes());
        // PBKDF2 always runs, even for an incorrect username. No lock is held over hashing.
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let valid = hash.verify(&password);
            valid && constant_time_equal(expected.as_ref(), actual.as_ref())
        })
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
    }

    pub(super) fn session(&self, headers: &HeaderMap) -> Option<Session> {
        let tokens = cookie_tokens(headers);
        if tokens.is_empty() {
            return None;
        }
        let mut sessions = self.inner.sessions.lock().ok()?;
        let now = Instant::now();
        let now_unix = unix_seconds();
        sessions.retain(|_, session| session.expires > now && session.expires_at > now_unix);
        for token in tokens {
            if let Some(found) = sessions.get(&token_hash(&token)).cloned() {
                return Some(found);
            }
        }
        None
    }

    pub(super) fn create_session(
        &self,
        old_headers: &HeaderMap,
    ) -> Result<(String, Session), StatusCode> {
        let token = random_token().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let csrf = random_token().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let session = Session {
            csrf,
            expires_at: unix_seconds() + self.inner.ttl.as_secs(),
            expires: Instant::now() + self.inner.ttl,
        };
        let mut sessions = self
            .inner
            .sessions
            .lock()
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let now = Instant::now();
        let now_unix = unix_seconds();
        sessions.retain(|_, s| s.expires > now && s.expires_at > now_unix);
        for old in cookie_tokens(old_headers) {
            sessions.remove(&token_hash(&old));
        }
        if sessions.len() >= MAX_SESSIONS {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        sessions.insert(token_hash(&token), session.clone());
        save_persisted_sessions(&sessions);
        Ok((token, session))
    }

    pub(super) fn logout(&self, headers: &HeaderMap) {
        let tokens = cookie_tokens(headers);
        if !tokens.is_empty() {
            if let Ok(mut sessions) = self.inner.sessions.lock() {
                for token in tokens {
                    sessions.remove(&token_hash(&token));
                }
                save_persisted_sessions(&sessions);
            }
        }
    }

    pub(super) fn session_value(&self, session: &Session) -> Value {
        json!({"user":{"username":self.inner.username,"role":"admin"},"csrf_token":session.csrf,"expires_at":session.expires_at})
    }

    pub(super) fn cookie(&self, token: &str) -> String {
        format!(
            "{COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}{}",
            if token.is_empty() {
                0
            } else {
                self.inner.ttl.as_secs()
            },
            if self.inner.secure_cookie {
                "; Secure"
            } else {
                ""
            }
        )
    }

    pub(super) fn clear_legacy_cookie(&self) -> String {
        format!(
            "{COOKIE}=; Path=/admin; HttpOnly; SameSite=Lax; Max-Age=0{}",
            if self.inner.secure_cookie {
                "; Secure"
            } else {
                ""
            }
        )
    }

    pub(super) fn audit(&self, action: &str, outcome: &str) {
        if let Ok(mut audit) = self.inner.audit.lock() {
            if audit.len() >= 500 {
                audit.pop_front();
            }
            // Never store bodies, tokens, passwords, raw errors or query parameters.
            audit.push_back(json!({"timestamp":unix_seconds(),"action":action,"outcome":outcome}));
        }
    }

    pub(super) fn audit_logs(&self) -> Value {
        let logs = self
            .inner
            .audit
            .lock()
            .map(|v| v.iter().rev().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        json!({"scope":"admin_audit","logs":logs,"persistent":false,"capacity":500,"request_logs_available":false})
    }
}

pub(super) fn csrf_valid(headers: &HeaderMap, session: &Session) -> bool {
    if headers.get_all("x-csrf-token").iter().count() != 1 {
        return false;
    }
    let Some(value) = headers.get("x-csrf-token").and_then(|v| v.to_str().ok()) else {
        return false;
    };
    // Compare fixed-size digests rather than variable-length secrets.
    constant_time_equal(&token_hash(value), &token_hash(&session.csrf))
}

fn forwarded_client(peer: IpAddr, headers: &HeaderMap, trusted: &HashSet<IpAddr>) -> IpAddr {
    if !trusted.contains(&peer) || headers.get_all("x-forwarded-for").iter().count() != 1 {
        return peer;
    }
    let Some(value) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) else {
        return peer;
    };
    if value.len() > 1024 {
        return peer;
    }
    let addresses = value
        .split(',')
        .map(|part| part.trim().parse::<IpAddr>())
        .collect::<Result<Vec<_>, _>>();
    let Ok(addresses) = addresses else {
        return peer;
    };
    if addresses.is_empty() || addresses.len() > 16 {
        return peer;
    }
    let mut current = peer;
    for address in addresses.into_iter().rev() {
        if !trusted.contains(&current) {
            break;
        }
        current = address;
    }
    current
}

#[allow(deprecated)]
fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    ring::constant_time::verify_slices_are_equal(left, right).is_ok()
}

fn cookie_tokens(headers: &HeaderMap) -> Vec<String> {
    let mut results = Vec::new();
    for cookie in headers.get_all("cookie") {
        let Ok(cookie) = cookie.to_str() else {
            continue;
        };
        for pair in cookie.split(';') {
            if let Some((name, value)) = pair.trim().split_once('=') {
                if name == COOKIE
                    && value.len() == 43
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                {
                    results.push(value.to_owned());
                }
            }
        }
    }
    results
}

fn sessions_file_path() -> std::path::PathBuf {
    let docker_data = std::path::Path::new("/home/data");
    if docker_data.is_dir() {
        return docker_data.join(".miora_sessions.json");
    }
    let local_dir = std::path::Path::new(".miora");
    if local_dir.is_dir() {
        return local_dir.join("sessions.json");
    }
    std::path::PathBuf::from(".miora_sessions.json")
}

fn load_persisted_sessions() -> HashMap<[u8; 32], Session> {
    let path = sessions_file_path();
    let Ok(raw) = std::fs::read(&path) else {
        return HashMap::new();
    };
    let Ok(entries) = serde_json::from_slice::<Vec<Value>>(&raw) else {
        return HashMap::new();
    };
    let now_unix = unix_seconds();
    let now_instant = Instant::now();
    let mut map = HashMap::new();
    for entry in entries {
        let Some(hash_hex) = entry.get("h").and_then(Value::as_str) else {
            continue;
        };
        let Some(csrf) = entry.get("c").and_then(Value::as_str) else {
            continue;
        };
        let Some(expires_at) = entry.get("e").and_then(Value::as_u64) else {
            continue;
        };
        if expires_at <= now_unix {
            continue;
        }
        let Ok(hash_bytes) = hex::decode(hash_hex) else {
            continue;
        };
        if hash_bytes.len() != 32 {
            continue;
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&hash_bytes);
        let remaining = Duration::from_secs(expires_at.saturating_sub(now_unix));
        map.insert(
            key,
            Session {
                csrf: csrf.to_owned(),
                expires_at,
                expires: now_instant + remaining,
            },
        );
    }
    map
}

fn save_persisted_sessions(sessions: &HashMap<[u8; 32], Session>) {
    let now_unix = unix_seconds();
    let entries: Vec<Value> = sessions
        .iter()
        .filter(|(_, s)| s.expires_at > now_unix)
        .map(|(k, s)| {
            json!({
                "h": hex::encode(k),
                "c": s.csrf,
                "e": s.expires_at,
            })
        })
        .collect();
    if let Ok(raw) = serde_json::to_vec(&entries) {
        let _ = std::fs::write(sessions_file_path(), raw);
    }
}

fn token_hash(token: &str) -> [u8; 32] {
    let hash = digest::digest(&digest::SHA256, token.as_bytes());
    let mut bytes = [0; 32];
    bytes.copy_from_slice(hash.as_ref());
    bytes
}

pub(super) fn random_token() -> Result<String, String> {
    let mut bytes = [0; 32];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| "Secure randomness unavailable")?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

pub(super) fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state() -> AdminState {
        AdminState::build(
            "admin".into(),
            None,
            true,
            Some("https://localhost".into()),
            Duration::from_secs(3600),
        )
    }
    #[test]
    fn hash_format_and_password_validation() {
        assert!(PasswordHash::parse("pbkdf2-sha256$1$00$00").is_err());
        assert!(PasswordHash::from_password("admin").is_err());
        let hash = PasswordHash::from_password("correct-long-password").unwrap();
        assert!(hash.verify("correct-long-password"));
        assert!(!hash.verify("wrong-long-password"));
    }
    #[test]
    fn session_rotation_logout_and_csrf() {
        let state = state();
        let (first, session) = state.create_session(&HeaderMap::new()).unwrap();
        assert_eq!(first.len(), 43);
        let mut headers = HeaderMap::new();
        headers.insert("cookie", format!("{COOKIE}={first}").parse().unwrap());
        assert!(state.session(&headers).is_some());
        assert!(!csrf_valid(&headers, &session));
        headers.insert("x-csrf-token", session.csrf.parse().unwrap());
        assert!(csrf_valid(&headers, &session));
        let (second, _) = state.create_session(&headers).unwrap();
        assert_ne!(first, second);
        assert!(state.session(&headers).is_none());
        headers.insert("cookie", format!("{COOKIE}={second}").parse().unwrap());
        state.logout(&headers);
        assert!(state.session(&headers).is_none());
        assert!(state.cookie(&second).contains("Path=/; HttpOnly; SameSite=Lax"));
        assert!(state.cookie(&second).ends_with("; Secure"));
    }
    #[test]
    fn expiry_duplicates_and_client_isolation() {
        let expired = AdminState::build("admin".into(), None, false, None, Duration::ZERO);
        let (token, _) = expired.create_session(&HeaderMap::new()).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("cookie", format!("{COOKIE}={token}").parse().unwrap());
        assert!(expired.session(&headers).is_none());
        let state = state();
        for _ in 0..LOGIN_ATTEMPTS {
            assert!(state.login_allowed("one-client"));
        }
        assert!(!state.login_allowed("one-client"));
        assert!(state.login_allowed("another-client"));
    }
    #[test]
    fn only_explicitly_trusted_proxy_hops_can_select_client_identity() {
        let peer: IpAddr = "127.0.0.1".parse().unwrap();
        let trusted = [peer, "10.0.0.1".parse().unwrap()].into_iter().collect();
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            "203.0.113.99, 192.0.2.2, 10.0.0.1".parse().unwrap(),
        );
        assert_eq!(
            forwarded_client(peer, &headers, &trusted),
            "192.0.2.2".parse::<IpAddr>().unwrap()
        );
        assert_eq!(forwarded_client(peer, &headers, &HashSet::new()), peer);
        headers.append("x-forwarded-for", "203.0.113.12".parse().unwrap());
        assert_eq!(forwarded_client(peer, &headers, &trusted), peer);
        headers.insert("x-forwarded-for", "not-an-address".parse().unwrap());
        assert_eq!(forwarded_client(peer, &headers, &trusted), peer);
    }
    #[test]
    fn origin_cannot_be_overridden_by_forwarded_headers() {
        let state = state();
        let mut headers = HeaderMap::new();
        headers.insert("origin", "https://attacker.example".parse().unwrap());
        headers.insert("x-forwarded-host", "attacker.example".parse().unwrap());
        assert!(!state.check_origin(&headers));
        headers.insert("origin", "https://localhost".parse().unwrap());
        assert!(state.check_origin(&headers));
        headers.insert("sec-fetch-site", "same-site".parse().unwrap());
        assert!(!state.check_origin(&headers));
    }
}
