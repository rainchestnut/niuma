//! Local file access policy and timeout-bounded TCC probes.
//!
//! Niuma sometimes needs to mirror desktop-local images into mobile transfers.
//! This module keeps that behavior explicit: startup probes configured
//! directories to trigger macOS TCC while the user is present, and task-time
//! reads only run for paths covered by a successful precheck.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::process::Command as TokioCommand;
use tokio::time;
use tracing::info;

use crate::cli::{FileAccessHelperArgs, FileAccessHelperCommands};

const DEFAULT_PRECHECK_TIMEOUT_SECONDS: u64 = 60;
const DEFAULT_READ_TIMEOUT_SECONDS: u64 = 10;
const MAX_PRECHECK_TIMEOUT_SECONDS: u64 = 120;
const MAX_READ_TIMEOUT_SECONDS: u64 = 15;
const HELPER_EXIT_PERMISSION_DENIED: i32 = 77;
const HELPER_EXIT_MISSING: i32 = 66;
#[cfg(not(test))]
const HELPER_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileAccessConfigFile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub precheck_roots: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub precheck_timeout_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileAccessConfig {
    pub precheck_roots: Vec<String>,
    pub precheck_timeout_seconds: u64,
    pub read_timeout_seconds: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileAccessSnapshot {
    pub roots_text: String,
    pub precheck_timeout_seconds: u64,
    pub read_timeout_seconds: u64,
    pub running: bool,
    pub updated_at: Option<i64>,
    pub roots: Vec<FileAccessRootSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileAccessRootSnapshot {
    pub root: String,
    pub normalized_root: Option<String>,
    pub status: FileAccessStatus,
    pub message: Option<String>,
    pub checked_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileAccessStatus {
    NotChecked,
    Checking,
    Granted,
    Denied,
    Timeout,
    Missing,
    Invalid,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileReadOutcome {
    Bytes(Vec<u8>),
    PathOnly { path: String, reason: String },
}

#[derive(Debug, Clone)]
struct FileAccessRoot {
    input: String,
    normalized_root: Option<PathBuf>,
    wildcard: bool,
    probe_paths: Vec<PathBuf>,
    status: FileAccessStatus,
    message: Option<String>,
    checked_at: Option<i64>,
}

#[derive(Debug, Clone)]
struct RuntimeState {
    config: FileAccessConfig,
    roots: Vec<FileAccessRoot>,
    running: bool,
    updated_at: Option<i64>,
    // Invalidates older async precheck tasks when the user changes config or reruns checks.
    generation: u64,
}

static RUNTIME: OnceLock<Arc<RwLock<RuntimeState>>> = OnceLock::new();

impl Default for FileAccessConfig {
    fn default() -> Self {
        Self {
            precheck_roots: vec!["~/Downloads".to_string(), "~/Documents".to_string()],
            precheck_timeout_seconds: DEFAULT_PRECHECK_TIMEOUT_SECONDS,
            read_timeout_seconds: DEFAULT_READ_TIMEOUT_SECONDS,
        }
    }
}

impl FileAccessConfig {
    pub fn from_config_file(file: Option<&FileAccessConfigFile>) -> Self {
        let defaults = Self::default();
        let Some(file) = file else {
            return defaults;
        };
        Self {
            precheck_roots: file
                .precheck_roots
                .clone()
                .unwrap_or(defaults.precheck_roots),
            precheck_timeout_seconds: clamp_timeout(
                file.precheck_timeout_seconds
                    .unwrap_or(defaults.precheck_timeout_seconds),
                1,
                MAX_PRECHECK_TIMEOUT_SECONDS,
            ),
            read_timeout_seconds: clamp_timeout(
                file.read_timeout_seconds
                    .unwrap_or(defaults.read_timeout_seconds),
                1,
                MAX_READ_TIMEOUT_SECONDS,
            ),
        }
    }

    pub fn into_config_file(self) -> FileAccessConfigFile {
        FileAccessConfigFile {
            precheck_roots: Some(self.precheck_roots),
            precheck_timeout_seconds: Some(self.precheck_timeout_seconds),
            read_timeout_seconds: Some(self.read_timeout_seconds),
        }
    }

    pub fn from_dashboard(
        roots_text: &str,
        precheck_timeout_seconds: u64,
        read_timeout_seconds: u64,
    ) -> Self {
        Self {
            precheck_roots: parse_roots_text(roots_text),
            precheck_timeout_seconds: clamp_timeout(
                precheck_timeout_seconds,
                1,
                MAX_PRECHECK_TIMEOUT_SECONDS,
            ),
            read_timeout_seconds: clamp_timeout(read_timeout_seconds, 1, MAX_READ_TIMEOUT_SECONDS),
        }
    }

    fn roots_text(&self) -> String {
        self.precheck_roots.join(";")
    }
}

pub fn parse_roots_text(value: &str) -> Vec<String> {
    value
        .split(';')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

pub fn configure(config: FileAccessConfig) {
    let roots = build_roots(&config, FileAccessStatus::NotChecked);
    let mut runtime = runtime().write().expect("file access runtime poisoned");
    let generation = runtime.generation.wrapping_add(1);
    *runtime = RuntimeState {
        config,
        roots,
        running: false,
        updated_at: None,
        generation,
    };
}

pub fn spawn_precheck() {
    let (config, generation) = {
        let mut runtime = runtime().write().expect("file access runtime poisoned");
        runtime.generation = runtime.generation.wrapping_add(1);
        runtime.running = true;
        runtime.updated_at = Some(unix_timestamp());
        runtime.roots = build_roots(&runtime.config, FileAccessStatus::Checking);
        (runtime.config.clone(), runtime.generation)
    };

    tokio::spawn(async move {
        let timeout = Duration::from_secs(config.precheck_timeout_seconds);
        let mut checked = Vec::new();
        for mut root in build_roots(&config, FileAccessStatus::Checking) {
            let result = precheck_root(&root, timeout).await;
            root.status = result.status;
            root.message = result.message;
            root.checked_at = Some(unix_timestamp());
            checked.push(root);
        }
        let mut runtime = runtime().write().expect("file access runtime poisoned");
        if runtime.generation != generation {
            return;
        }
        runtime.roots = checked;
        runtime.running = false;
        runtime.updated_at = Some(unix_timestamp());
        info!("file access precheck finished");
    });
}

pub fn snapshot() -> FileAccessSnapshot {
    let runtime = runtime().read().expect("file access runtime poisoned");
    FileAccessSnapshot {
        roots_text: runtime.config.roots_text(),
        precheck_timeout_seconds: runtime.config.precheck_timeout_seconds,
        read_timeout_seconds: runtime.config.read_timeout_seconds,
        running: runtime.running,
        updated_at: runtime.updated_at,
        roots: runtime
            .roots
            .iter()
            .map(|root| FileAccessRootSnapshot {
                root: root.input.clone(),
                normalized_root: root
                    .normalized_root
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned()),
                status: root.status,
                message: root.message.clone(),
                checked_at: root.checked_at,
            })
            .collect(),
    }
}

pub fn read_file_for_transfer(path: &Path) -> FileReadOutcome {
    let path_text = path.to_string_lossy().into_owned();
    let runtime = runtime().read().expect("file access runtime poisoned");
    let Some(root) = runtime
        .roots
        .iter()
        .find(|root| root_matches_path(root, path))
    else {
        return FileReadOutcome::PathOnly {
            path: path_text,
            reason: "路径不在启动预检查目录中".to_string(),
        };
    };
    if root.status != FileAccessStatus::Granted {
        return FileReadOutcome::PathOnly {
            path: path_text,
            reason: format!("目录权限状态为 {}", status_label(root.status)),
        };
    }
    #[cfg(not(test))]
    let timeout = Duration::from_secs(runtime.config.read_timeout_seconds);
    drop(runtime);

    #[cfg(test)]
    {
        match std::fs::read(path) {
            Ok(bytes) => FileReadOutcome::Bytes(bytes),
            Err(error) => FileReadOutcome::PathOnly {
                path: path_text,
                reason: format!("读取失败：{error}"),
            },
        }
    }

    #[cfg(not(test))]
    match helper_copy_with_timeout(path, timeout) {
        Ok(bytes) => FileReadOutcome::Bytes(bytes),
        Err(reason) => FileReadOutcome::PathOnly {
            path: path_text,
            reason,
        },
    }
}

pub fn run_helper(args: FileAccessHelperArgs) -> Result<()> {
    match args.command {
        FileAccessHelperCommands::Precheck { path } => {
            helper_precheck_path(&path);
        }
        FileAccessHelperCommands::Copy { path, output } => {
            helper_copy_file(&path, &output);
        }
    }
}

#[cfg(test)]
pub fn configure_test_roots(roots: Vec<String>, status: FileAccessStatus) {
    let config = FileAccessConfig {
        precheck_roots: roots,
        precheck_timeout_seconds: DEFAULT_PRECHECK_TIMEOUT_SECONDS,
        read_timeout_seconds: DEFAULT_READ_TIMEOUT_SECONDS,
    };
    let roots = build_roots(&config, status);
    let mut runtime = runtime().write().expect("file access runtime poisoned");
    let generation = runtime.generation.wrapping_add(1);
    *runtime = RuntimeState {
        config,
        roots,
        running: false,
        updated_at: Some(unix_timestamp()),
        generation,
    };
}

fn runtime() -> &'static Arc<RwLock<RuntimeState>> {
    RUNTIME.get_or_init(|| {
        let config = FileAccessConfig::default();
        Arc::new(RwLock::new(RuntimeState {
            roots: build_roots(&config, FileAccessStatus::NotChecked),
            config,
            running: false,
            updated_at: None,
            generation: 0,
        }))
    })
}

fn build_roots(config: &FileAccessConfig, status: FileAccessStatus) -> Vec<FileAccessRoot> {
    config
        .precheck_roots
        .iter()
        .map(|input| build_root(input, status))
        .collect()
}

fn build_root(input: &str, status: FileAccessStatus) -> FileAccessRoot {
    let trimmed = input.trim();
    if trimmed == "/*" {
        let probe_paths = default_protected_probe_paths();
        return FileAccessRoot {
            input: trimmed.to_string(),
            normalized_root: Some(PathBuf::from("/")),
            wildcard: true,
            probe_paths,
            status,
            message: None,
            checked_at: None,
        };
    }

    let normalized = expand_home(trimmed).map(PathBuf::from);
    let probe_paths = normalized.iter().cloned().collect();
    FileAccessRoot {
        input: trimmed.to_string(),
        normalized_root: normalized,
        wildcard: false,
        probe_paths,
        status,
        message: None,
        checked_at: None,
    }
}

struct PrecheckResult {
    status: FileAccessStatus,
    message: Option<String>,
}

async fn precheck_root(root: &FileAccessRoot, timeout: Duration) -> PrecheckResult {
    if root.normalized_root.is_none() {
        return PrecheckResult {
            status: FileAccessStatus::Invalid,
            message: Some("目录必须是绝对路径、~ 开头路径或 /*".to_string()),
        };
    }
    if root.probe_paths.is_empty() {
        return PrecheckResult {
            status: FileAccessStatus::Missing,
            message: Some("没有可检查的目录".to_string()),
        };
    }

    let mut last_missing = None;
    for path in &root.probe_paths {
        match helper_precheck_with_timeout(path, timeout).await {
            Ok(()) => {}
            Err(PrecheckFailure::Denied(message)) => {
                return PrecheckResult {
                    status: FileAccessStatus::Denied,
                    message: Some(message),
                };
            }
            Err(PrecheckFailure::Timeout) => {
                return PrecheckResult {
                    status: FileAccessStatus::Timeout,
                    message: Some(format!("权限请求超过 {} 秒未完成", timeout.as_secs())),
                };
            }
            Err(PrecheckFailure::Missing(message)) => {
                last_missing = Some(message);
            }
            Err(PrecheckFailure::Error(message)) => {
                return PrecheckResult {
                    status: FileAccessStatus::Error,
                    message: Some(message),
                };
            }
        }
    }

    if let Some(message) = last_missing {
        PrecheckResult {
            status: FileAccessStatus::Missing,
            message: Some(message),
        }
    } else {
        PrecheckResult {
            status: FileAccessStatus::Granted,
            message: None,
        }
    }
}

enum PrecheckFailure {
    Denied(String),
    Timeout,
    Missing(String),
    Error(String),
}

async fn helper_precheck_with_timeout(
    path: &Path,
    timeout: Duration,
) -> Result<(), PrecheckFailure> {
    let mut child = TokioCommand::new(current_exe().map_err(PrecheckFailure::Error)?)
        .arg("file-access-helper")
        .arg("precheck")
        .arg("--path")
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| PrecheckFailure::Error(error.to_string()))?;

    match time::timeout(timeout, child.wait()).await {
        Ok(Ok(status)) if status.success() => Ok(()),
        Ok(Ok(status)) if status.code() == Some(HELPER_EXIT_PERMISSION_DENIED) => Err(
            PrecheckFailure::Denied(format!("没有访问 {} 的权限", path.display())),
        ),
        Ok(Ok(status)) if status.code() == Some(HELPER_EXIT_MISSING) => Err(
            PrecheckFailure::Missing(format!("{} 不存在或不是目录", path.display())),
        ),
        Ok(Ok(status)) => Err(PrecheckFailure::Error(format!(
            "权限检查失败，退出码 {:?}",
            status.code()
        ))),
        Ok(Err(error)) => Err(PrecheckFailure::Error(error.to_string())),
        Err(_) => {
            let _ = child.kill().await;
            Err(PrecheckFailure::Timeout)
        }
    }
}

#[cfg(not(test))]
fn helper_copy_with_timeout(path: &Path, timeout: Duration) -> Result<Vec<u8>, String> {
    use std::process::Command;
    use std::time::Instant;

    let cache_dir = crate::paths::state_root()
        .map_err(|error| error.to_string())?
        .join("file-access")
        .join("read-cache");
    std::fs::create_dir_all(&cache_dir).map_err(|error| error.to_string())?;
    let output_path = cache_dir.join(format!("{}.bin", uuid::Uuid::new_v4()));
    let mut child = Command::new(current_exe().map_err(|error| error.to_string())?)
        .arg("file-access-helper")
        .arg("copy")
        .arg("--path")
        .arg(path)
        .arg("--output")
        .arg(&output_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&output_path);
            return Err(format!("文件读取超过 {} 秒未完成", timeout.as_secs()));
        }
        std::thread::sleep(HELPER_POLL_INTERVAL);
    };

    if !status.success() {
        let _ = std::fs::remove_file(&output_path);
        return match status.code() {
            Some(HELPER_EXIT_PERMISSION_DENIED) => Err("没有文件访问权限".to_string()),
            Some(HELPER_EXIT_MISSING) => Err("文件不存在".to_string()),
            code => Err(format!("文件读取失败，退出码 {code:?}")),
        };
    }

    let bytes = std::fs::read(&output_path).map_err(|error| error.to_string())?;
    let _ = std::fs::remove_file(&output_path);
    Ok(bytes)
}

fn helper_precheck_path(path: &Path) -> ! {
    if !path.exists() || !path.is_dir() {
        std::process::exit(HELPER_EXIT_MISSING);
    }
    match std::fs::read_dir(path) {
        Ok(mut entries) => match entries.next().transpose() {
            Ok(_) => std::process::exit(0),
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                std::process::exit(HELPER_EXIT_PERMISSION_DENIED)
            }
            Err(_) => std::process::exit(1),
        },
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            std::process::exit(HELPER_EXIT_PERMISSION_DENIED)
        }
        Err(_) => std::process::exit(1),
    }
}

fn helper_copy_file(path: &Path, output: &Path) -> ! {
    if !path.exists() || !path.is_file() {
        std::process::exit(HELPER_EXIT_MISSING);
    }
    let result = std::fs::read(path).and_then(|bytes| {
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temp = output.with_extension("tmp");
        std::fs::write(&temp, bytes)?;
        std::fs::rename(temp, output)
    });
    match result {
        Ok(()) => std::process::exit(0),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            std::process::exit(HELPER_EXIT_PERMISSION_DENIED)
        }
        Err(_) => std::process::exit(1),
    }
}

fn root_matches_path(root: &FileAccessRoot, path: &Path) -> bool {
    if root.wildcard {
        return path.is_absolute();
    }
    root.normalized_root
        .as_ref()
        .is_some_and(|prefix| path.starts_with(prefix))
}

fn expand_home(path: &str) -> Option<String> {
    if path == "~" {
        return home_dir().map(|home| home.to_string_lossy().into_owned());
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return home_dir().map(|home| home.join(rest).to_string_lossy().into_owned());
    }
    let path = PathBuf::from(path);
    path.is_absolute()
        .then(|| path.to_string_lossy().into_owned())
}

fn default_protected_probe_paths() -> Vec<PathBuf> {
    let Some(home) = home_dir() else {
        return Vec::new();
    };
    ["Downloads", "Documents"]
        .into_iter()
        .map(|name| home.join(name))
        .filter(|path| path.exists())
        .collect()
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn current_exe() -> std::result::Result<PathBuf, String> {
    std::env::current_exe().map_err(|error| error.to_string())
}

fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn status_label(status: FileAccessStatus) -> &'static str {
    match status {
        FileAccessStatus::NotChecked => "not_checked",
        FileAccessStatus::Checking => "checking",
        FileAccessStatus::Granted => "granted",
        FileAccessStatus::Denied => "denied",
        FileAccessStatus::Timeout => "timeout",
        FileAccessStatus::Missing => "missing",
        FileAccessStatus::Invalid => "invalid",
        FileAccessStatus::Error => "error",
    }
}

fn clamp_timeout(value: u64, min: u64, max: u64) -> u64 {
    value.clamp(min, max)
}

pub fn configure_from_gateway(config: &FileAccessConfig) {
    configure(config.clone());
    spawn_precheck();
    info!(
        roots = %config.roots_text(),
        precheck_timeout_seconds = config.precheck_timeout_seconds,
        read_timeout_seconds = config.read_timeout_seconds,
        "file access precheck started"
    );
}
