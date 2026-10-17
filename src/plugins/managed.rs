use super::manifest::Manifest;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncReadExt;

pub(super) type Logs = Arc<Mutex<VecDeque<String>>>;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Package {
    pub directory: String,
    pub requirements: Vec<String>,
}

pub(crate) struct Process {
    child: Mutex<Child>,
    pub logs: Logs,
}

impl Process {
    pub fn running(&self) -> bool {
        matches!(self.child.lock().unwrap().try_wait(), Ok(None))
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let child = self.child.get_mut().unwrap();
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    command.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
}

fn python() -> String {
    std::env::var("MIORA_PLUGIN_PYTHON")
        .unwrap_or_else(|_| if cfg!(windows) { "python" } else { "python3" }.into())
}

fn environment_python(directory: &Path) -> PathBuf {
    directory.join(if cfg!(windows) {
        ".venv/Scripts/python.exe"
    } else {
        ".venv/bin/python"
    })
}

fn append(logs: &Logs, bytes: &[u8]) {
    let mut logs = logs.lock().unwrap();
    let mut text = String::from_utf8_lossy(bytes).into_owned();
    if text.len() > 2048 {
        let mut boundary = 2048;
        while !text.is_char_boundary(boundary) {
            boundary -= 1;
        }
        text.truncate(boundary);
    }
    logs.push_back(text);
    while logs.len() > 200 {
        logs.pop_front();
    }
}

async fn drain(mut input: impl tokio::io::AsyncRead + Unpin, logs: Logs) {
    let mut buffer = [0u8; 2048];
    while let Ok(count) = input.read(&mut buffer).await {
        if count == 0 {
            break;
        }
        append(&logs, &buffer[..count]);
    }
}

async fn run(mut command: Command, logs: &Logs) -> Result<(), String> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = tokio::process::Command::from(command)
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("start Python setup: {error}"))?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let completion = async {
        let (status, (), ()) = tokio::join!(
            child.wait(),
            drain(stdout, logs.clone()),
            drain(stderr, logs.clone())
        );
        status
    };
    let status = tokio::time::timeout(Duration::from_secs(300), completion)
        .await
        .map_err(|_| "Python setup timed out".to_owned())?
        .map_err(|error| format!("wait for Python setup: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Python setup failed ({status}); see installation log"
        ))
    }
}

pub(super) struct Staging {
    pub path: PathBuf,
    pub name: String,
    keep: bool,
}

impl Staging {
    pub fn new(root: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(root)
            .map_err(|error| format!("create plugin package directory: {error}"))?;
        let root = root
            .canonicalize()
            .map_err(|error| format!("resolve plugin package directory: {error}"))?;
        let name = format!(
            "package-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let path = root.join(&name);
        std::fs::create_dir(&path)
            .map_err(|error| format!("create plugin staging directory: {error}"))?;
        Ok(Self {
            path,
            name,
            keep: false,
        })
    }
    pub fn retain(&mut self) {
        self.keep = true;
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

pub(super) fn package_path(root: &Path, package: &Package) -> Result<PathBuf, String> {
    if !package.directory.starts_with("package-")
        || !package
            .directory
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err("invalid managed package directory".into());
    }
    Ok(root.join(&package.directory))
}

pub(super) async fn unpack(
    bytes: &[u8],
    staging: &Staging,
    extract: bool,
) -> Result<Value, String> {
    let archive = staging.path.join("upload.zip");
    tokio::fs::write(&archive, bytes)
        .await
        .map_err(|error| format!("write plugin upload: {error}"))?;
    let mut process = command(python());
    process
        .args(["-I", "-c", include_str!("managed_unpack.py")])
        .arg(&archive);
    if extract {
        process.arg(staging.path.join("source"));
    }
    let output = tokio::time::timeout(
        Duration::from_secs(30),
        tokio::process::Command::from(process)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "plugin unpack timed out".to_owned())?
    .map_err(|error| format!("run plugin unpacker: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("decode plugin manifest: {error}"))?;
    let manifest: Manifest = serde_json::from_value(value["manifest"].clone())
        .map_err(|error| format!("read plugin manifest: {error}"))?;
    manifest.validate()?;
    Ok(value)
}

pub(super) async fn prepare(staging: &Staging, logs: &Logs) -> Result<(), String> {
    let mut create = command(python());
    create
        .args(["-m", "venv"])
        .arg(staging.path.join("source/.venv"));
    run(create, logs).await?;
    let directory = staging.path.join("source");
    let requirements = directory.join("requirements.txt");
    if requirements.is_file() {
        let mut pip = command(environment_python(&directory));
        pip.args([
            "-m",
            "pip",
            "install",
            "--disable-pip-version-check",
            "--no-input",
            "-r",
        ])
        .arg(requirements);
        run(pip, logs).await?;
    }
    let sdk = directory.join("__miora__");
    tokio::fs::create_dir(&sdk)
        .await
        .map_err(|error| format!("create plugin SDK directory: {error}"))?;
    tokio::fs::write(
        sdk.join("miora_plugin.py"),
        include_str!("../../sdk/python/miora_plugin.py"),
    )
    .await
    .map_err(|error| format!("install plugin SDK: {error}"))?;
    tokio::fs::write(
        sdk.join("miora_resources.py"),
        include_str!("../../sdk/python/miora_resources.py"),
    )
    .await
    .map_err(|error| format!("install plugin resource SDK: {error}"))?;
    tokio::fs::remove_file(staging.path.join("upload.zip"))
        .await
        .map_err(|error| format!("remove staged archive: {error}"))?;
    Ok(())
}

pub(super) async fn start(
    root: &Path,
    package: &Package,
    id: &str,
    token: &str,
    logs: Logs,
) -> Result<(Arc<Process>, String), String> {
    let directory = package_path(root, package)?.join("source");
    let state = root
        .parent()
        .ok_or("missing plugin store")?
        .join("credentials")
        .join(id);
    std::fs::create_dir_all(&state)
        .map_err(|error| format!("create plugin credentials directory: {error}"))?;
    let ready = directory.join("__miora__/ready.json");
    if ready.exists() {
        std::fs::remove_file(&ready)
            .map_err(|error| format!("clear plugin ready state: {error}"))?;
    }
    let mut child = command(environment_python(&directory));
    child.current_dir(&directory)
        .args(["-u", "-c", "import runpy,sys; sys.path.insert(0,'__miora__'); runpy.run_path('plugin.py',run_name='__main__')"])
        .env("MIORA_PLUGIN_SERVICE_TOKEN", token)
        .env("MIORA_PLUGIN_READY_FILE", &ready)
        .env("MIORA_PLUGIN_PARENT_PID", std::process::id().to_string())
        .env("MIORA_PLUGIN_STATE_DIR", state)
        .stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = child
        .spawn()
        .map_err(|error| format!("start managed plugin: {error}"))?;
    for mut input in [
        child
            .stdout
            .take()
            .map(|pipe| Box::new(pipe) as Box<dyn std::io::Read + Send>),
        child
            .stderr
            .take()
            .map(|pipe| Box::new(pipe) as Box<dyn std::io::Read + Send>),
    ]
    .into_iter()
    .flatten()
    {
        let logs = logs.clone();
        std::thread::spawn(move || {
            let mut buffer = [0u8; 2048];
            while let Ok(count) = input.read(&mut buffer) {
                if count == 0 {
                    break;
                }
                append(&logs, &buffer[..count]);
            }
        });
    }
    let process = Arc::new(Process {
        child: Mutex::new(child),
        logs,
    });
    for _ in 0..150 {
        if !process.running() {
            return Err("managed plugin exited before becoming ready".into());
        }
        if let Ok(bytes) = tokio::fs::read(&ready).await {
            if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
                if let Some(port) = value["port"]
                    .as_u64()
                    .filter(|port| *port > 0 && *port <= 65535)
                {
                    return Ok((process, format!("http://127.0.0.1:{port}")));
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err("managed plugin did not become ready within 15 seconds".into())
}
