use serde::Deserialize;
use sentia_protocol::{INFERENCE_SOCKET_PATH_V1, LOCAL_BROKER_SOCKET_PATH_V1};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BrokerConfig {
    pub version: u32,
    pub listen_socket: PathBuf,
    pub backend_socket: PathBuf,
    pub backend_model: String,
    pub minimum_peer_uid: u32,
    pub maximum_peer_uid: u32,
    pub max_active: usize,
    pub max_global_queue: usize,
    pub max_queue_per_uid: usize,
    pub max_request_bytes: usize,
    pub max_messages: usize,
    pub max_prompt_bytes: usize,
    pub max_generation_tokens: u32,
    pub backend_connect_timeout_ms: u64,
    pub backend_request_timeout_ms: u64,
}

impl Default for BrokerConfig {
    fn default() -> Self {
        Self {
            version: 1,
            listen_socket: PathBuf::from(LOCAL_BROKER_SOCKET_PATH_V1),
            backend_socket: PathBuf::from(INFERENCE_SOCKET_PATH_V1),
            backend_model: "sentia-local".to_owned(),
            minimum_peer_uid: 1000,
            maximum_peer_uid: 60_000,
            max_active: 1,
            max_global_queue: 16,
            max_queue_per_uid: 2,
            max_request_bytes: 1024 * 1024,
            max_messages: 64,
            max_prompt_bytes: 512 * 1024,
            max_generation_tokens: 2048,
            backend_connect_timeout_ms: 30_000,
            backend_request_timeout_ms: 180_000,
        }
    }
}

impl BrokerConfig {
    pub fn load(path: Option<&Path>) -> io::Result<Self> {
        let path = path
            .map(Path::to_path_buf)
            .or_else(|| env::var_os("SENTIA_LOCAL_BROKER_CONFIG").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("/etc/sentia/local-broker.json"));
        let mut config = if path.exists() {
            serde_json::from_slice::<Self>(&fs::read(path)?)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
        } else {
            Self::default()
        };
        if let Some(path) = env::var_os("SENTIA_LOCAL_BROKER_SOCKET") {
            config.listen_socket = path.into();
        }
        if let Some(path) = env::var_os("SENTIA_LLAMA_SOCKET") {
            config.backend_socket = path.into();
        }
        config.validate()?;
        Ok(config)
    }

    fn validate(&mut self) -> io::Result<()> {
        if self.version != 1
            || self.minimum_peer_uid == 0
            || self.maximum_peer_uid < self.minimum_peer_uid
            || self.max_active == 0
            || self.max_active > 4
            || self.max_global_queue == 0
            || self.max_queue_per_uid == 0
            || self.max_queue_per_uid > self.max_global_queue
            || self.max_request_bytes == 0
            || self.max_request_bytes > 8 * 1024 * 1024
            || self.max_messages == 0
            || self.max_messages > 256
            || self.max_prompt_bytes == 0
            || self.max_generation_tokens == 0
            || self.max_generation_tokens > 8192
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "local broker configuration limits are invalid",
            ));
        }
        Ok(())
    }

    pub fn connect_timeout(&self) -> Duration {
        Duration::from_millis(self.backend_connect_timeout_ms)
    }

    pub fn request_timeout(&self) -> Duration {
        Duration::from_millis(self.backend_request_timeout_ms)
    }
}
