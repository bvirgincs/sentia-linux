use crate::{
    config::BrokerConfig,
    llama::{LlamaClient, LlamaError},
    queue::{InferenceQueue, QueueError},
};
use sentia_local_broker::protocol::{
    BrokerEvent, BrokerMessage, BrokerRequest, BrokerTool, BROKER_PROTOCOL_VERSION,
};
use sentia_protocol::{
    ContractError, ContractErrorCode, ProviderErrorCode, ProviderHealth,
};
use std::{
    collections::HashMap,
    fs, io,
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    path::Path,
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::{mpsc, Mutex},
    task::JoinSet,
    time::{sleep, timeout, Instant},
};
use tokio_util::sync::CancellationToken;

pub struct Broker {
    config: Arc<BrokerConfig>,
    llama: LlamaClient,
    queue: InferenceQueue,
}

impl Broker {
    pub fn new(config: BrokerConfig) -> Self {
        let config = Arc::new(config);
        Self {
            llama: LlamaClient::new(config.clone()),
            queue: InferenceQueue::new(
                config.max_active,
                config.max_global_queue,
                config.max_queue_per_uid,
            ),
            config,
        }
    }

    pub async fn run(self: Arc<Self>, shutdown: CancellationToken) -> io::Result<()> {
        let listener = bind_public_socket(&self.config.listen_socket)?;
        let mut connections = JoinSet::new();
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => break,
                accepted = listener.accept() => {
                    let (stream, _) = accepted?;
                    let broker = self.clone();
                    let connection_shutdown = shutdown.child_token();
                    connections.spawn(async move {
                        if let Err(error) = broker.handle_connection(stream, connection_shutdown).await {
                            eprintln!("sentia-local-broker: connection ended: {error}");
                        }
                    });
                }
                Some(result) = connections.join_next(), if !connections.is_empty() => {
                    if let Err(error) = result {
                        eprintln!("sentia-local-broker: connection task failed: {error}");
                    }
                }
            }
        }
        shutdown.cancel();
        while connections.join_next().await.is_some() {}
        if self.config.listen_socket.exists() {
            let metadata = fs::symlink_metadata(&self.config.listen_socket)?;
            if metadata.file_type().is_socket() && metadata.uid() == unsafe { libc::geteuid() } {
                let _ = fs::remove_file(&self.config.listen_socket);
            }
        }
        Ok(())
    }

    async fn handle_connection(
        self: Arc<Self>,
        stream: UnixStream,
        shutdown: CancellationToken,
    ) -> io::Result<()> {
        let uid = validate_peer(&stream, self.config.minimum_peer_uid)?;
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let (event_tx, mut event_rx) = mpsc::channel::<BrokerEvent>(128);
        let writer_shutdown = shutdown.clone();
        let writer_task = tokio::spawn(async move {
            loop {
                let event = tokio::select! {
                    _ = writer_shutdown.cancelled() => break,
                    event = event_rx.recv() => match event {
                        Some(value) => value,
                        None => break,
                    }
                };
                let mut bytes = serde_json::to_vec(&event)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                bytes.push(b'\n');
                writer.write_all(&bytes).await?;
            }
            writer.shutdown().await
        });
        let active = Arc::new(Mutex::new(HashMap::<String, CancellationToken>::new()));
        let mut tasks = JoinSet::new();

        loop {
            let frame = tokio::select! {
                _ = shutdown.cancelled() => break,
                value = read_frame(&mut reader, self.config.max_request_bytes) => value,
            };
            let frame = match frame {
                Ok(Some(value)) => value,
                Ok(None) => break,
                Err(error) => {
                    send_error(
                        &event_tx,
                        "unknown",
                        ContractErrorCode::InvalidRequest,
                        ProviderErrorCode::MalformedResponse,
                        error.to_string(),
                        false,
                    )
                    .await;
                    break;
                }
            };
            let request: BrokerRequest = match serde_json::from_slice(&frame) {
                Ok(value) => value,
                Err(error) => {
                    send_error(
                        &event_tx,
                        "unknown",
                        ContractErrorCode::InvalidRequest,
                        ProviderErrorCode::MalformedResponse,
                        format!("malformed broker request: {error}"),
                        false,
                    )
                    .await;
                    continue;
                }
            };
            match request {
                BrokerRequest::Health {
                    version,
                    request_id,
                } => {
                    if !valid_envelope(version, &request_id, &event_tx).await {
                        continue;
                    }
                    let health = timeout(Duration::from_secs(10), self.llama.health()).await;
                    let (state, detail) = match health {
                        Ok(Ok(value)) => value,
                        Ok(Err(error)) => (ProviderHealth::Unavailable, error.message),
                        Err(_) => (
                            ProviderHealth::Unavailable,
                            "local inference health check timed out".to_owned(),
                        ),
                    };
                    let _ = event_tx
                        .send(BrokerEvent::Health {
                            version: BROKER_PROTOCOL_VERSION.to_owned(),
                            request_id,
                            health: state,
                            detail,
                        })
                        .await;
                }
                BrokerRequest::Cancel {
                    version,
                    request_id,
                    target_request_id,
                } => {
                    if !valid_envelope(version, &request_id, &event_tx).await {
                        continue;
                    }
                    if let Some(token) = active.lock().await.get(&target_request_id) {
                        token.cancel();
                    }
                    let _ = event_tx
                        .send(BrokerEvent::Cancelled {
                            version: BROKER_PROTOCOL_VERSION.to_owned(),
                            request_id,
                            target_request_id,
                        })
                        .await;
                }
                BrokerRequest::Chat {
                    version,
                    request_id,
                    messages,
                    tools,
                    max_tokens,
                } => {
                    if !valid_envelope(version, &request_id, &event_tx).await {
                        continue;
                    }
                    if let Err(message) =
                        validate_chat(&self.config, &messages, &tools, max_tokens)
                    {
                        send_error(
                            &event_tx,
                            &request_id,
                            ContractErrorCode::InvalidRequest,
                            ProviderErrorCode::UnsupportedCapability,
                            message,
                            false,
                        )
                        .await;
                        continue;
                    }
                    if active.lock().await.contains_key(&request_id) {
                        send_error(
                            &event_tx,
                            &request_id,
                            ContractErrorCode::InvalidRequest,
                            ProviderErrorCode::MalformedResponse,
                            "request_id is already active on this connection",
                            false,
                        )
                        .await;
                        continue;
                    }
                    let (reservation, position) = match self.queue.reserve(uid) {
                        Ok(value) => value,
                        Err(error) => {
                            send_error(
                                &event_tx,
                                &request_id,
                                ContractErrorCode::ProviderUnavailable,
                                ProviderErrorCode::Quota,
                                error.to_string(),
                                true,
                            )
                            .await;
                            continue;
                        }
                    };
                    let _ = event_tx
                        .send(BrokerEvent::Queued {
                            version: BROKER_PROTOCOL_VERSION.to_owned(),
                            request_id: request_id.clone(),
                            position,
                        })
                        .await;
                    let cancellation = shutdown.child_token();
                    active
                        .lock()
                        .await
                        .insert(request_id.clone(), cancellation.clone());
                    let broker = self.clone();
                    let output = event_tx.clone();
                    let active_requests = active.clone();
                    tasks.spawn(async move {
                        broker
                            .run_chat(
                                request_id.clone(),
                                messages,
                                tools,
                                max_tokens,
                                reservation,
                                output,
                                cancellation,
                            )
                            .await;
                        active_requests.lock().await.remove(&request_id);
                    });
                }
            }
            while tasks.try_join_next().is_some() {}
        }
        shutdown.cancel();
        for token in active.lock().await.values() {
            token.cancel();
        }
        while tasks.join_next().await.is_some() {}
        drop(event_tx);
        writer_task
            .await
            .map_err(|error| io::Error::new(io::ErrorKind::Other, error))?
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_chat(
        &self,
        request_id: String,
        messages: Vec<BrokerMessage>,
        tools: Vec<BrokerTool>,
        max_tokens: u32,
        mut reservation: crate::queue::Reservation,
        events: mpsc::Sender<BrokerEvent>,
        cancellation: CancellationToken,
    ) {
        if let Err(error) = reservation.acquire(&cancellation).await {
            if matches!(error, QueueError::Cancelled) {
                let _ = events
                    .send(BrokerEvent::Cancelled {
                        version: BROKER_PROTOCOL_VERSION.to_owned(),
                        request_id: request_id.clone(),
                        target_request_id: request_id,
                    })
                    .await;
            } else {
                send_error(
                    &events,
                    &request_id,
                    ContractErrorCode::ProviderUnavailable,
                    ProviderErrorCode::Quota,
                    error.to_string(),
                    true,
                )
                .await;
            }
            return;
        }
        let _ = events
            .send(BrokerEvent::Progress {
                version: BROKER_PROTOCOL_VERSION.to_owned(),
                request_id: request_id.clone(),
                state: "starting".to_owned(),
                detail: "Local AI starting".to_owned(),
            })
            .await;
        if let Err(error) = self
            .wait_until_ready(&request_id, &events, cancellation.clone())
            .await
        {
            if cancellation.is_cancelled() {
                send_cancelled(&events, &request_id).await;
            } else {
                send_llama_error(&events, &request_id, error).await;
            }
            return;
        }
        let _ = events
            .send(BrokerEvent::Progress {
                version: BROKER_PROTOCOL_VERSION.to_owned(),
                request_id: request_id.clone(),
                state: "ready".to_owned(),
                detail: "Local AI ready".to_owned(),
            })
            .await;
        let result = timeout(
            self.config.request_timeout(),
            self.llama.chat(
                &request_id,
                &messages,
                &tools,
                max_tokens,
                &events,
                cancellation.clone(),
            ),
        )
        .await;
        match result {
            Ok(Ok(output)) if output.tool_calls.is_empty() => {
                let _ = events
                    .send(BrokerEvent::Complete {
                        version: BROKER_PROTOCOL_VERSION.to_owned(),
                        request_id,
                        finish_reason: output.finish_reason,
                    })
                    .await;
            }
            Ok(Ok(output)) => {
                let _ = events
                    .send(BrokerEvent::ToolCalls {
                        version: BROKER_PROTOCOL_VERSION.to_owned(),
                        request_id,
                        calls: output.tool_calls,
                    })
                    .await;
            }
            Ok(Err(_error)) if cancellation.is_cancelled() => {
                send_cancelled(&events, &request_id).await
            }
            Ok(Err(error)) => send_llama_error(&events, &request_id, error).await,
            Err(_) => {
                cancellation.cancel();
                send_error(
                    &events,
                    &request_id,
                    ContractErrorCode::Timeout,
                    ProviderErrorCode::Timeout,
                    "local inference request exceeded its time limit",
                    true,
                )
                .await;
            }
        }
    }

    async fn wait_until_ready(
        &self,
        request_id: &str,
        events: &mpsc::Sender<BrokerEvent>,
        cancellation: CancellationToken,
    ) -> Result<(), LlamaError> {
        let deadline = Instant::now() + self.config.connect_timeout();
        loop {
            let health_result = tokio::select! {
                _ = cancellation.cancelled() => {
                    return Err(LlamaError {
                        kind: ProviderErrorCode::Internal,
                        message: "local inference request cancelled".to_owned(),
                        retryable: false,
                    });
                }
                result = self.llama.health() => result,
            };
            let health = match health_result {
                Ok(health) => health,
                Err(error) if error.retryable && Instant::now() < deadline => {
                    let _ = events
                        .send(BrokerEvent::Progress {
                            version: BROKER_PROTOCOL_VERSION.to_owned(),
                            request_id: request_id.to_owned(),
                            state: "backend_starting".to_owned(),
                            detail: "Waiting for the shared local inference service".to_owned(),
                        })
                        .await;
                    tokio::select! {
                        _ = cancellation.cancelled() => {
                            return Err(LlamaError {
                                kind: ProviderErrorCode::Internal,
                                message: "local inference request cancelled".to_owned(),
                                retryable: false,
                            });
                        }
                        _ = sleep(Duration::from_millis(250)) => {}
                    }
                    continue;
                }
                Err(error) => return Err(error),
            };
            match health.0 {
                ProviderHealth::Healthy => return Ok(()),
                ProviderHealth::Starting if Instant::now() < deadline => {
                    let _ = events
                        .send(BrokerEvent::Progress {
                            version: BROKER_PROTOCOL_VERSION.to_owned(),
                            request_id: request_id.to_owned(),
                            state: "model_loading".to_owned(),
                            detail: "Waiting for the shared local model".to_owned(),
                        })
                        .await;
                    tokio::select! {
                        _ = cancellation.cancelled() => {
                            return Err(LlamaError {
                                kind: ProviderErrorCode::Internal,
                                message: "local inference request cancelled".to_owned(),
                                retryable: false,
                            });
                        }
                        _ = sleep(Duration::from_millis(250)) => {}
                    }
                }
                ProviderHealth::Starting => {
                    return Err(LlamaError {
                        kind: ProviderErrorCode::Timeout,
                        message: "shared local model did not become ready in time".to_owned(),
                        retryable: true,
                    });
                }
                _ => {
                    return Err(LlamaError {
                        kind: ProviderErrorCode::ProcessFailure,
                        message: health.1,
                        retryable: true,
                    });
                }
            }
        }
    }
}

async fn send_cancelled(events: &mpsc::Sender<BrokerEvent>, request_id: &str) {
    let _ = events
        .send(BrokerEvent::Cancelled {
            version: BROKER_PROTOCOL_VERSION.to_owned(),
            request_id: request_id.to_owned(),
            target_request_id: request_id.to_owned(),
        })
        .await;
}

fn validate_chat(
    config: &BrokerConfig,
    messages: &[BrokerMessage],
    tools: &[BrokerTool],
    max_tokens: u32,
) -> Result<(), &'static str> {
    if messages.is_empty() || messages.len() > config.max_messages {
        return Err("message count is outside configured bounds");
    }
    let total = messages
        .iter()
        .try_fold(0_usize, |total, message| {
            if !matches!(message.role.as_str(), "system" | "user" | "assistant" | "tool")
                || message.role.len() > 16
                || message.name.as_ref().map(String::len).unwrap_or(0) > 128
                || message.tool_call_id.as_ref().map(String::len).unwrap_or(0) > 128
            {
                return None;
            }
            total.checked_add(message.content.len())
        })
        .ok_or("messages contain invalid fields")?;
    if total > config.max_prompt_bytes {
        return Err("prompt exceeds configured size limit");
    }
    if max_tokens == 0 || max_tokens > config.max_generation_tokens {
        return Err("max_tokens is outside configured bounds");
    }
    if tools.len() > 64
        || tools.iter().any(|tool| {
            tool.name.is_empty()
                || tool.name.len() > 128
                || tool.description.len() > 4096
                || tool.parameters.to_string().len() > 64 * 1024
        })
    {
        return Err("tool definitions are outside configured bounds");
    }
    Ok(())
}

async fn valid_envelope(
    version: String,
    request_id: &str,
    events: &mpsc::Sender<BrokerEvent>,
) -> bool {
    if version != BROKER_PROTOCOL_VERSION {
        send_error(
            events,
            request_id,
            ContractErrorCode::UnsupportedVersion,
            ProviderErrorCode::UnsupportedCapability,
            "unsupported broker protocol version",
            false,
        )
        .await;
        return false;
    }
    if request_id.is_empty() || request_id.len() > 128 {
        send_error(
            events,
            "unknown",
            ContractErrorCode::InvalidRequest,
            ProviderErrorCode::MalformedResponse,
            "request_id must contain 1 to 128 bytes",
            false,
        )
        .await;
        return false;
    }
    true
}

async fn send_llama_error(
    events: &mpsc::Sender<BrokerEvent>,
    request_id: &str,
    error: LlamaError,
) {
    let code = match error.kind {
        ProviderErrorCode::Timeout => ContractErrorCode::Timeout,
        _ => ContractErrorCode::ProviderUnavailable,
    };
    send_error(
        events,
        request_id,
        code,
        error.kind,
        error.message,
        error.retryable,
    )
    .await;
}

async fn send_error(
    events: &mpsc::Sender<BrokerEvent>,
    request_id: &str,
    code: ContractErrorCode,
    provider_error: ProviderErrorCode,
    message: impl Into<String>,
    retryable: bool,
) {
    let _ = events
        .send(BrokerEvent::Error {
            version: BROKER_PROTOCOL_VERSION.to_owned(),
            request_id: request_id.to_owned(),
            error: ContractError::new(code, message, retryable),
            provider_error,
        })
        .await;
}

async fn read_frame<R>(reader: &mut R, maximum: usize) -> io::Result<Option<Vec<u8>>>
where
    R: AsyncBufRead + Unpin,
{
    let mut output = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return if output.is_empty() {
                Ok(None)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "unterminated JSONL broker frame",
                ))
            };
        }
        let take = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| index + 1)
            .unwrap_or(available.len());
        if output.len().saturating_add(take) > maximum {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "broker frame exceeds configured size limit",
            ));
        }
        output.extend_from_slice(&available[..take]);
        let complete = available[take - 1] == b'\n';
        reader.consume(take);
        if complete {
            output.pop();
            if output.last() == Some(&b'\r') {
                output.pop();
            }
            if output.is_empty() {
                continue;
            }
            return Ok(Some(output));
        }
    }
}

fn validate_peer(stream: &UnixStream, minimum_uid: u32) -> io::Result<u32> {
    let credentials = stream.peer_cred()?;
    let uid = credentials.uid();
    if uid < minimum_uid || uid == unsafe { libc::geteuid() } {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "local broker rejected non-login or broker peer UID",
        ));
    }
    Ok(uid)
}

fn bind_public_socket(path: &Path) -> io::Result<UnixListener> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "broker socket has no parent")
    })?;
    if parent.exists() {
        let metadata = fs::symlink_metadata(parent)?;
        if metadata.file_type().is_symlink()
            || !metadata.is_dir()
            || metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "broker runtime directory is not an owned directory",
            ));
        }
    } else {
        fs::create_dir_all(parent)?;
    }
    fs::set_permissions(parent, fs::Permissions::from_mode(0o755))?;
    if path.exists() {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.file_type().is_socket() || metadata.uid() != unsafe { libc::geteuid() } {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "refusing to replace non-owned broker socket path",
            ));
        }
        fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o660))?;
    Ok(listener)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[tokio::test]
    async fn public_socket_permissions_are_exact() {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/test-state")
            .join(format!("broker-socket-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let socket = directory.join("broker.sock");
        let _listener = bind_public_socket(&socket).unwrap();
        assert_eq!(
            fs::metadata(&socket).unwrap().permissions().mode() & 0o777,
            0o660
        );
        fs::remove_file(socket).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[tokio::test]
    async fn malformed_frames_are_bounded() {
        let mut input = BufReader::new(&b"abcdef\n"[..]);
        assert!(read_frame(&mut input, 4).await.is_err());
    }
}
