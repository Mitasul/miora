use crate::config::source::RuntimeConfigPublisher;
use crate::runtime::context::{env_bool, AppState};
use axum::routing::any;
use axum::Router;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let public_port = env_u16("PORT", 8000)?;
    let database_disabled = env_bool("DISABLE_DATABASE", false);
    bootstrap_admin_config()?;
    let publisher = RuntimeConfigPublisher::discover(database_disabled)?;
    publisher.publish().await?;
    publisher.start_watcher();
    let persistence = crate::storage::database::Persistence::initialize(database_disabled).await?;
    let state = AppState::new(String::new(), String::new(), false, persistence, publisher)?;
    let _ = state.runtime.refresh().await;
    state.runtime.start_watcher();

    state.runtime.restore_controls_on_start().await?;

    let app = Router::new()
        .fallback(any(crate::api::handler::handler))
        .with_state(state);
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), public_port);
    let listener = tokio::net::TcpListener::bind(address).await?;
    crate::observability::backend_logs::event(
        serde_json::json!({"event_type":"runtime_started","message":format!("Miora runtime listening on {address}")}),
    );
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    Ok(())
}

/// Explicit opt-in for a fresh admin-managed installation. Never overwrites an
/// existing source and never creates a default inference key or admin password.
fn bootstrap_admin_config() -> Result<(), String> {
    if !env_bool("MIORA_BOOTSTRAP_EMPTY_CONFIG", false) {
        return Ok(());
    }
    if !crate::admin::AdminState::from_env()?.configured() {
        return Err("MIORA_BOOTSTRAP_EMPTY_CONFIG requires administrator credentials".into());
    }
    if std::env::var("CONFIG_URL").is_ok_and(|url| !url.trim().is_empty()) {
        return Err("MIORA_BOOTSTRAP_EMPTY_CONFIG cannot be combined with CONFIG_URL".into());
    }
    let path = std::path::PathBuf::from(
        std::env::var("UNI_API_CONFIG_PATH").unwrap_or_else(|_| "api.yaml".into()),
    );
    if path.is_file() {
        return Ok(());
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create config directory: {error}"))?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .map_err(|error| format!("create empty configuration: {error}"))?;
    use std::io::Write;
    file.write_all(b"providers: []\napi_keys: []\n")
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("initialize empty configuration: {error}"))
}

fn env_u16(name: &str, default: u16) -> Result<u16, String> {
    let raw = std::env::var(name).unwrap_or_else(|_| default.to_string());
    raw.parse::<u16>()
        .map_err(|_| format!("{name} must be a valid TCP port"))
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut terminate = signal(SignalKind::terminate()).expect("install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = terminate.recv() => {},
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
