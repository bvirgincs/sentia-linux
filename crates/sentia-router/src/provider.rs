use crate::api::FailureKind;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sentia_local_broker::protocol::{
    BrokerEvent, BrokerMessage, BrokerRequest, BrokerTool, BrokerToolCall,
    BROKER_PROTOCOL_VERSION, MAX_BROKER_FRAME_BYTES,
};
use sentia_protocol::{
    BoundedString, ProviderErrorCode, ProviderHealth, ProviderState, ProviderStatus,
};
use std::{
    collections::HashMap,
    fs, io,
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
    sync::mpsc,
    time::timeout,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ModelMessage {
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub tool_call_id: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<BrokerToolCall>,
}

#[derive(Clone, Debug)]
pub struct ProviderRequest {
    pub request_id: String,
    pub messages: Vec<ModelMessage>,
    pub tools: Vec<BrokerTool>,
    pub max_tokens: u32,
    pub boundary: RequestBoundary,
}

#[derive(Clone, Debug)]
pub enum RequestBoundary {
    LocalOnly,
    SanitizedRemote { payload_sha256: String },
}

#[derive(Clone, Debug)]
pub enum ProviderChunk {
    Progress { state: String, detail: String },
    Text(String),
}

#[derive(Clone, Debug)]
pub struct ProviderOutput {
    pub finish_reason: String,
    pub tool_calls: Vec<BrokerToolCall>,
}

#[derive(Clone, Debug, thiserror::Error)]
#[error("{message}")]
pub struct ProviderError {
    pub kind: FailureKind,
    pub message: String,
    pub retryable: bool,
    pub emitted_output: bool,
}

impl ProviderError {
    pub fn new(kind: FailureKind, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            kind,
            message: message.into(),
            retryable,
            emitted_output: false,
        }
    }
}

#[async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    fn is_local(&self) -> bool;

    async fn health(&self) -> ProviderStatus;

    async fn chat(
        &self,
        request: ProviderRequest,
        chunks: mpsc::Sender<ProviderChunk>,
        cancellation: CancellationToken,
    ) -> Result<ProviderOutput, ProviderError>;
}

#[derive(Clone)]
pub struct LocalBrokerProvider {
    socket: PathBuf,
    timeout: Duration,
}

/// Decide whether the broker socket is safe to use before connecting.
///
/// The socket is deliberately mode `0666`: the broker is the machine-wide entry
/// point for every login session and authorises callers from `SO_PEERCRED`, not
/// from the filesystem mode. The substitution defence is therefore the parent
/// directory, which must be owned by the same non-root service account and must
/// not be group- or world-writable, so no other user can replace the socket.
///
/// This is a pure function because the mode is stated in three places - the
/// broker's bind, this check, and the router contract defaults - and a silent
/// disagreement between them disables all local AI while every unit still
/// reports healthy.
fn broker_socket_is_safe(
    is_socket: bool,
    socket_uid: u32,
    socket_mode: u32,
    parent_uid: u32,
    parent_mode: u32,
    self_uid: u32,
) -> bool {
    is_socket
        && socket_uid != 0
        && socket_uid != self_uid
        && socket_mode & 0o777 == 0o666
        && parent_uid == socket_uid
        && parent_mode & 0o022 == 0
}

impl LocalBrokerProvider {
    pub fn new(socket: PathBuf, timeout: Duration) -> Self {
        Self { socket, timeout }
    }

    async fn connect(&self) -> Result<UnixStream, ProviderError> {
        let metadata = fs::symlink_metadata(&self.socket).map_err(io_provider_error)?;
        let parent = self.socket.parent().ok_or_else(|| {
            ProviderError::new(
                FailureKind::Provider(ProviderErrorCode::ProcessFailure),
                "local broker socket path has no parent",
                false,
            )
        })?;
        let parent_metadata = fs::symlink_metadata(parent).map_err(io_provider_error)?;
        if !broker_socket_is_safe(
            metadata.file_type().is_socket(),
            metadata.uid(),
            metadata.permissions().mode(),
            parent_metadata.uid(),
            parent_metadata.permissions().mode(),
            unsafe { libc::geteuid() },
        ) {
            return Err(ProviderError::new(
                FailureKind::Provider(ProviderErrorCode::ProcessFailure),
                "local broker socket ownership or directory permissions are unsafe",
                false,
            ));
        }
        let stream = timeout(
            self.timeout.min(Duration::from_secs(10)),
            UnixStream::connect(&self.socket),
        )
            .await
            .map_err(|_| {
                ProviderError::new(
                    FailureKind::Provider(ProviderErrorCode::Timeout),
                    "timed out connecting to local AI broker",
                    true,
                )
            })?
            .map_err(|error| {
                ProviderError::new(
                    FailureKind::Provider(ProviderErrorCode::ProcessFailure),
                    format!("local AI broker unavailable: {error}"),
                    true,
                )
            })?;
        let peer = stream.peer_cred().map_err(io_provider_error)?;
        if peer.uid() != metadata.uid() {
            return Err(ProviderError::new(
                FailureKind::Provider(ProviderErrorCode::ProcessFailure),
                "local broker peer credentials did not match socket ownership",
                false,
            ));
        }
        Ok(stream)
    }
}

#[async_trait]
impl Provider for LocalBrokerProvider {
    fn name(&self) -> &str {
        "local"
    }

    fn is_local(&self) -> bool {
        true
    }

    async fn health(&self) -> ProviderStatus {
        let request_id = format!("health-{}", std::process::id());
        let result = async {
            let mut stream = self.connect().await?;
            write_json_line(
                &mut stream,
                &BrokerRequest::Health {
                    version: BROKER_PROTOCOL_VERSION.to_owned(),
                    request_id: request_id.clone(),
                },
            )
            .await
            .map_err(io_provider_error)?;
            let mut reader = BufReader::new(stream);
            let event: BrokerEvent = read_json_line(&mut reader).await?;
            match event {
                BrokerEvent::Health { health, detail, .. } => Ok((health, detail)),
                BrokerEvent::Error { error, .. } => {
                    Ok((ProviderHealth::Unavailable, error.message))
                }
                _ => Err(ProviderError::new(
                    FailureKind::Provider(ProviderErrorCode::MalformedResponse),
                    "local broker returned an unexpected health response",
                    false,
                )),
            }
        }
        .await;
        match result {
            Ok((health, detail)) => provider_status(self.name(), health, detail),
            Err(error) => {
                provider_status(self.name(), ProviderHealth::Unavailable, error.message)
            }
        }
    }

    async fn chat(
        &self,
        request: ProviderRequest,
        chunks: mpsc::Sender<ProviderChunk>,
        cancellation: CancellationToken,
    ) -> Result<ProviderOutput, ProviderError> {
        if !matches!(request.boundary, RequestBoundary::LocalOnly) {
            return Err(ProviderError::new(
                FailureKind::Provider(ProviderErrorCode::Internal),
                "local provider rejected a remote-egress request boundary",
                false,
            ));
        }
        let mut stream = self.connect().await?;
        let broker_request = BrokerRequest::Chat {
            version: BROKER_PROTOCOL_VERSION.to_owned(),
            request_id: request.request_id.clone(),
            messages: request
                .messages
                .into_iter()
                .map(|value| BrokerMessage {
                    role: value.role,
                    content: value.content,
                    name: value.name,
                    tool_call_id: value.tool_call_id,
                    tool_calls: value.tool_calls,
                })
                .collect(),
            tools: request.tools,
            max_tokens: request.max_tokens,
        };
        write_json_line(&mut stream, &broker_request)
            .await
            .map_err(io_provider_error)?;
        let mut reader = BufReader::new(stream);
        let mut emitted_output = false;
        let result = timeout(self.timeout, async {
            loop {
                let event = tokio::select! {
                    _ = cancellation.cancelled() => {
                        return Err(ProviderError::new(
                            FailureKind::Cancelled,
                            "local inference cancelled",
                            false,
                        ));
                    }
                    event = read_json_line::<_, BrokerEvent>(&mut reader) => event?,
                };
                match event {
                    BrokerEvent::Queued { position, .. } => {
                        let _ = chunks
                            .send(ProviderChunk::Progress {
                                state: "queued".to_owned(),
                                detail: format!("Local AI queue position {position}"),
                            })
                            .await;
                    }
                    BrokerEvent::Progress { state, detail, .. } => {
                        let _ = chunks.send(ProviderChunk::Progress { state, detail }).await;
                    }
                    BrokerEvent::Delta { content, .. } => {
                        emitted_output = true;
                        if chunks.send(ProviderChunk::Text(content)).await.is_err() {
                            return Err(ProviderError::new(
                                FailureKind::Cancelled,
                                "request receiver closed",
                                false,
                            ));
                        }
                    }
                    BrokerEvent::ToolCalls { calls, .. } => {
                        return Ok(ProviderOutput {
                            finish_reason: "tool_calls".to_owned(),
                            tool_calls: calls,
                        });
                    }
                    BrokerEvent::Complete { finish_reason, .. } => {
                        return Ok(ProviderOutput {
                            finish_reason,
                            tool_calls: Vec::new(),
                        });
                    }
                    BrokerEvent::Error {
                        provider_error,
                        error,
                        ..
                    } => {
                        return Err(ProviderError {
                            kind: FailureKind::Provider(provider_error),
                            message: error.message,
                            retryable: error.retryable,
                            emitted_output,
                        });
                    }
                    BrokerEvent::Cancelled { .. } => {
                        return Err(ProviderError {
                            kind: FailureKind::Cancelled,
                            message: "local inference cancelled".to_owned(),
                            retryable: false,
                            emitted_output,
                        });
                    }
                    BrokerEvent::Health { .. } => {
                        return Err(ProviderError::new(
                            FailureKind::Provider(ProviderErrorCode::MalformedResponse),
                            "unexpected health event during chat",
                            false,
                        ));
                    }
                }
            }
        })
        .await;
        match result {
            Ok(Ok(output)) => Ok(output),
            Ok(Err(mut error)) => {
                error.emitted_output |= emitted_output;
                Err(error)
            }
            Err(_) => Err(ProviderError {
                kind: FailureKind::Provider(ProviderErrorCode::Timeout),
                message: "local inference timed out".to_owned(),
                retryable: true,
                emitted_output,
            }),
        }
    }
}

async fn write_json_line<T: Serialize>(stream: &mut UnixStream, value: &T) -> io::Result<()> {
    let mut bytes =
        serde_json::to_vec(value).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    bytes.push(b'\n');
    stream.write_all(&bytes).await
}

async fn read_json_line<R, T>(reader: &mut R) -> Result<T, ProviderError>
where
    R: tokio::io::AsyncBufRead + Unpin,
    T: serde::de::DeserializeOwned,
{
    let mut bytes = Vec::new();
    loop {
        let available = reader.fill_buf().await.map_err(io_provider_error)?;
        if available.is_empty() {
            return Err(ProviderError::new(
                FailureKind::Provider(ProviderErrorCode::ProcessFailure),
                "local broker closed the connection",
                true,
            ));
        }
        let take = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| index + 1)
            .unwrap_or(available.len());
        if bytes.len().saturating_add(take) > MAX_BROKER_FRAME_BYTES {
            return Err(ProviderError::new(
                FailureKind::Provider(ProviderErrorCode::MalformedResponse),
                "local broker response exceeded size limit",
                false,
            ));
        }
        bytes.extend_from_slice(&available[..take]);
        let complete = available[take - 1] == b'\n';
        reader.consume(take);
        if complete {
            break;
        }
    }
    serde_json::from_slice(&bytes).map_err(|error| {
        ProviderError::new(
            FailureKind::Provider(ProviderErrorCode::MalformedResponse),
            format!("invalid local broker response: {error}"),
            false,
        )
    })
}

fn io_provider_error(error: io::Error) -> ProviderError {
    ProviderError::new(
        FailureKind::Provider(ProviderErrorCode::ProcessFailure),
        format!("local broker I/O failed: {error}"),
        true,
    )
}

fn provider_status(
    provider: &str,
    health: ProviderHealth,
    message: String,
) -> ProviderStatus {
    ProviderStatus {
        provider_id: BoundedString::new(provider)
            .expect("built-in provider identifier is within protocol bounds"),
        state: match &health {
            ProviderHealth::Healthy => ProviderState::Idle,
            ProviderHealth::Starting => ProviderState::Warming,
            ProviderHealth::Degraded => ProviderState::Backoff,
            ProviderHealth::Unavailable | ProviderHealth::Disabled => ProviderState::Failed,
        },
        health,
        checked_at_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
        message: Some(message),
        observed_latency_ms: None,
    }
}

#[derive(Default)]
pub struct ProviderRegistry {
    local: Option<Arc<dyn Provider>>,
    remotes: HashMap<String, Arc<dyn Provider>>,
}

impl ProviderRegistry {
    pub fn with_local(local: Arc<dyn Provider>) -> Self {
        Self {
            local: Some(local),
            remotes: HashMap::new(),
        }
    }

    pub fn local(&self) -> Option<Arc<dyn Provider>> {
        self.local.clone()
    }

    pub fn register_remote(&mut self, provider: Arc<dyn Provider>) {
        if !provider.is_local() {
            self.remotes.insert(provider.name().to_owned(), provider);
        }
    }

    pub fn remote(&self, name: &str) -> Option<Arc<dyn Provider>> {
        self.remotes.get(name).cloned()
    }

    pub fn remote_providers(&self) -> Vec<Arc<dyn Provider>> {
        self.remotes.values().cloned().collect()
    }
}

#[derive(Clone)]
pub struct CircuitBreakers {
    states: Arc<Mutex<HashMap<String, CircuitState>>>,
    threshold: u32,
    cooldown: Duration,
}

#[derive(Clone, Copy, Debug)]
struct CircuitState {
    failures: u32,
    open_until: Option<Instant>,
}

impl CircuitBreakers {
    pub fn new(threshold: u32, cooldown: Duration) -> Self {
        Self {
            states: Arc::new(Mutex::new(HashMap::new())),
            threshold,
            cooldown,
        }
    }

    pub fn allow(&self, provider: &str) -> bool {
        let mut states = self.states.lock().expect("circuit lock poisoned");
        let Some(state) = states.get_mut(provider) else {
            return true;
        };
        if let Some(deadline) = state.open_until {
            if Instant::now() < deadline {
                return false;
            }
            state.open_until = None;
            state.failures = 0;
        }
        true
    }

    pub fn success(&self, provider: &str) {
        self.states.lock().expect("circuit lock poisoned").remove(provider);
    }

    pub fn failure(&self, provider: &str) {
        let mut states = self.states.lock().expect("circuit lock poisoned");
        let state = states.entry(provider.to_owned()).or_insert(CircuitState {
            failures: 0,
            open_until: None,
        });
        state.failures = state.failures.saturating_add(1);
        if state.failures >= self.threshold {
            state.open_until = Some(Instant::now() + self.cooldown);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circuit_opens_after_threshold() {
        let breakers = CircuitBreakers::new(2, Duration::from_secs(10));
        assert!(breakers.allow("remote"));
        breakers.failure("remote");
        assert!(breakers.allow("remote"));
        breakers.failure("remote");
        assert!(!breakers.allow("remote"));
        breakers.success("remote");
        assert!(breakers.allow("remote"));
    }

    #[test]
    fn broker_socket_accepts_the_shipped_service_layout() {
        // /run/sentia-local is RuntimeDirectory 0755 owned by sentia-inference,
        // and the broker binds broker.sock 0666 inside it.
        assert!(broker_socket_is_safe(true, 992, 0o666, 992, 0o755, 1000));
    }

    #[test]
    fn broker_socket_rejects_unsafe_layouts() {
        // Not a socket at all.
        assert!(!broker_socket_is_safe(false, 992, 0o666, 992, 0o755, 1000));
        // Root-owned, which the broker never is.
        assert!(!broker_socket_is_safe(true, 0, 0o666, 0, 0o755, 1000));
        // Our own socket, which would mean the broker is not running.
        assert!(!broker_socket_is_safe(true, 1000, 0o666, 1000, 0o755, 1000));
        // A mode no login session can open: the regression this guards.
        assert!(!broker_socket_is_safe(true, 992, 0o660, 992, 0o755, 1000));
        // A directory owned by somebody else, or writable by anybody else,
        // would let a third party substitute the socket.
        assert!(!broker_socket_is_safe(true, 992, 0o666, 993, 0o755, 1000));
        assert!(!broker_socket_is_safe(true, 992, 0o666, 992, 0o757, 1000));
        assert!(!broker_socket_is_safe(true, 992, 0o666, 992, 0o775, 1000));
    }
}
