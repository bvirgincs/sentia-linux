use anyhow::{anyhow, Context, Result};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const ROUTER_SOCKET_RELATIVE_PATH: &str = "sentia/router.sock";
pub const HEALTH_SOCKET_DEFAULT_PATH: &str = "/run/sentia-health/metrics.sock";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RouterPolicy {
    LocalOnly,
    LocalPreferred,
    RemotePreferred,
    AskBeforeRemote,
}

impl RouterPolicy {
    pub fn as_contract_label(self) -> &'static str {
        match self {
            RouterPolicy::LocalOnly => "LOCAL_ONLY",
            RouterPolicy::LocalPreferred => "LOCAL_PREFERRED",
            RouterPolicy::RemotePreferred => "REMOTE_PREFERRED",
            RouterPolicy::AskBeforeRemote => "ASK_BEFORE_REMOTE",
        }
    }
}

impl std::fmt::Display for RouterPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_contract_label())
    }
}

pub fn router_socket_path() -> PathBuf {
    if let Some(explicit) = env::var_os("SENTIA_ROUTER_SOCKET") {
        return PathBuf::from(explicit);
    }

    let runtime_dir = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe { libc::geteuid() })));

    runtime_dir.join(ROUTER_SOCKET_RELATIVE_PATH)
}

pub fn health_socket_path() -> PathBuf {
    env::var_os("SENTIA_HEALTH_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(HEALTH_SOCKET_DEFAULT_PATH))
}

pub fn verify_router_socket_0600(socket_path: &Path) -> Result<()> {
    let metadata = fs::metadata(socket_path)
        .with_context(|| format!("failed to inspect router socket at {}", socket_path.display()))?;

    if !metadata.file_type().is_socket() {
        return Err(anyhow!(
            "router endpoint {} is not a unix socket",
            socket_path.display()
        ));
    }

    let mode = metadata.permissions().mode() & 0o777;
    if mode != 0o600 {
        return Err(anyhow!(
            "router socket {} must be mode 0600, found {:o}",
            socket_path.display(),
            mode
        ));
    }

    let parent = socket_path.parent().ok_or_else(|| {
        anyhow!(
            "router socket {} has no parent directory",
            socket_path.display()
        )
    })?;
    let parent_mode = fs::metadata(parent)?.permissions().mode() & 0o777;
    if parent_mode & 0o077 != 0 {
        return Err(anyhow!(
            "router socket parent {} must not be accessible by group/others (mode {:o})",
            parent.display(),
            parent_mode
        ));
    }

    Ok(())
}

pub trait RouterClient {
    fn send_request(&self, request_json: &str) -> std::io::Result<String>;
}

#[derive(Debug, Clone)]
pub struct UnixRouterClient {
    pub socket_path: PathBuf,
    pub timeout: Duration,
}

impl Default for UnixRouterClient {
    fn default() -> Self {
        Self {
            socket_path: router_socket_path(),
            timeout: Duration::from_secs(3),
        }
    }
}

impl RouterClient for UnixRouterClient {
    fn send_request(&self, request_json: &str) -> std::io::Result<String> {
        let mut stream = UnixStream::connect(&self.socket_path)?;
        stream.set_read_timeout(Some(self.timeout))?;
        stream.set_write_timeout(Some(self.timeout))?;
        stream.write_all(request_json.as_bytes())?;
        stream.write_all(b"\n")?;

        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        Ok(response)
    }
}
