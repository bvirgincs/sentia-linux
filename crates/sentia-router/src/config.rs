use crate::api::MAX_FRAME_BYTES;
use serde::{Deserialize, Serialize};
use sentia_protocol::LOCAL_BROKER_SOCKET_PATH_V1;
use std::{
    env,
    fs,
    io,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RouterConfig {
    pub version: u32,
    pub broker_socket: PathBuf,
    pub tool_socket: Option<PathBuf>,
    pub health_socket: Option<PathBuf>,
    pub max_request_bytes: usize,
    pub max_connections: usize,
    pub max_inflight_per_connection: usize,
    pub max_agent_rounds: u32,
    pub max_generation_tokens: u32,
    pub provider_timeout_ms: u64,
    pub circuit_failure_threshold: u32,
    pub circuit_cooldown_ms: u64,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            version: 1,
            broker_socket: PathBuf::from(LOCAL_BROKER_SOCKET_PATH_V1),
            tool_socket: None,
            health_socket: None,
            max_request_bytes: MAX_FRAME_BYTES,
            max_connections: 64,
            max_inflight_per_connection: 8,
            max_agent_rounds: 4,
            max_generation_tokens: 2048,
            provider_timeout_ms: 120_000,
            circuit_failure_threshold: 3,
            circuit_cooldown_ms: 60_000,
        }
    }
}

impl RouterConfig {
    pub fn load(path: Option<&Path>) -> io::Result<Self> {
        let path = path
            .map(Path::to_path_buf)
            .or_else(|| env::var_os("SENTIA_ROUTER_CONFIG").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("/etc/sentia/router-runtime.json"));
        let mut config = if path.exists() {
            serde_json::from_slice::<Self>(&fs::read(path)?)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
        } else {
            Self::default()
        };
        config.validate()?;
        if let Some(path) = env::var_os("SENTIA_BROKER_SOCKET") {
            config.broker_socket = path.into();
        }
        if let Some(path) = env::var_os("SENTIA_TOOL_SOCKET") {
            config.tool_socket = Some(path.into());
        }
        if let Some(path) = env::var_os("SENTIA_HEALTH_SOCKET") {
            config.health_socket = Some(path.into());
        }
        Ok(config)
    }

    pub fn validate(&mut self) -> io::Result<()> {
        if self.version != 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unsupported router configuration version",
            ));
        }
        if self.max_request_bytes == 0 || self.max_request_bytes > 8 * 1024 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "max_request_bytes must be between 1 and 8388608",
            ));
        }
        if self.max_connections == 0
            || self.max_inflight_per_connection == 0
            || self.max_agent_rounds == 0
            || self.max_agent_rounds > 8
            || self.max_generation_tokens == 0
            || self.max_generation_tokens > 8192
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "router limits are outside supported bounds",
            ));
        }
        Ok(())
    }

    pub fn provider_timeout(&self) -> Duration {
        Duration::from_millis(self.provider_timeout_ms)
    }

    pub fn circuit_cooldown(&self) -> Duration {
        Duration::from_millis(self.circuit_cooldown_ms)
    }
}

pub fn runtime_socket_path() -> io::Result<PathBuf> {
    if let Some(path) = env::var_os("SENTIA_ROUTER_SOCKET") {
        return Ok(path.into());
    }
    let runtime = env::var_os("XDG_RUNTIME_DIR").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "XDG_RUNTIME_DIR is required for the per-user router socket",
        )
    })?;
    Ok(PathBuf::from(runtime).join("sentia/router.sock"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn packaged_contract_config_is_valid() {
        let config: sentia_protocol::RouterConfig = serde_json::from_slice(include_bytes!(
            "../../../config/router/router.json"
        ))
        .unwrap();
        config.validate().unwrap();
    }
}
