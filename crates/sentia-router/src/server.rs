use crate::{
    api::{
        ChatContent, ChatInput, ControlRequest, ControlResponse, InternalEvent,
        CHAT_MIME, CONSENT_MIME, CONTROL_MIME, MAX_FRAME_BYTES, SETTINGS_MIME, STATUS_MIME,
    },
    config::runtime_socket_path,
    router::{error_event, Router},
};
use sentia_protocol::{
    BoundedString, CancelAck, ContractError, ContractErrorCode, EventPayload, JsonlFrame,
    ProviderHealth, ProviderState, ProviderStatus, RequestStatus, ResultPayload, RouterCapability,
    RouterRequest, RouterResult, StreamEvent, ToolName, ToolPhase, PROTOCOL_VERSION_V1,
};
use sentia_protocol::router::is_valid_status_transition;
use std::{
    collections::HashMap,
    env, fs, io,
    os::{
        fd::{FromRawFd, RawFd},
        unix::{
            fs::{FileTypeExt, MetadataExt, PermissionsExt},
            net::UnixListener as StdUnixListener,
        },
    },
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::{mpsc, Mutex, Semaphore},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

static FRAME_SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub async fn run(router: Arc<Router>, shutdown: CancellationToken) -> io::Result<()> {
    let listener = acquire_listener(listener_from_systemd, bind_runtime_socket)?;
    let connection_limit = Arc::new(Semaphore::new(router.config().max_connections));
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let Ok(permit) = connection_limit.clone().try_acquire_owned() else {
                    drop(stream);
                    continue;
                };
                let router = router.clone();
                let connection_shutdown = shutdown.child_token();
                connections.spawn(async move {
                    let _permit = permit;
                    if let Err(error) = handle_connection(stream, router, connection_shutdown).await {
                        eprintln!("sentia-router: client connection ended: {error}");
                    }
                });
            }
            Some(result) = connections.join_next(), if !connections.is_empty() => {
                if let Err(error) = result {
                    eprintln!("sentia-router: client task failed: {error}");
                }
            }
        }
    }
    shutdown.cancel();
    while connections.join_next().await.is_some() {}
    Ok(())
}

async fn handle_connection(
    stream: UnixStream,
    router: Arc<Router>,
    shutdown: CancellationToken,
) -> io::Result<()> {
    validate_peer(&stream)?;
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let (frame_tx, mut frame_rx) = mpsc::channel::<JsonlFrame>(128);
    let writer_shutdown = shutdown.clone();
    let writer_task = tokio::spawn(async move {
        loop {
            let frame = tokio::select! {
                _ = writer_shutdown.cancelled() => break,
                frame = frame_rx.recv() => match frame {
                    Some(frame) => frame,
                    None => break,
                }
            };
            let mut bytes = serde_json::to_vec(&frame)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            bytes.push(b'\n');
            writer.write_all(&bytes).await?;
        }
        writer.shutdown().await
    });
    let active = Arc::new(Mutex::new(HashMap::<String, CancellationToken>::new()));
    let inflight = Arc::new(Semaphore::new(
        router.config().max_inflight_per_connection,
    ));
    let mut requests = JoinSet::new();

    loop {
        let frame = tokio::select! {
            _ = shutdown.cancelled() => break,
            frame = read_frame(&mut reader, router.config().max_request_bytes) => frame,
        };
        let frame = match frame {
            Ok(Some(frame)) => frame,
            Ok(None) => break,
            Err(error) => {
                send_protocol_error(
                    &frame_tx,
                    "unknown",
                    "unknown",
                    ContractError::new(
                        ContractErrorCode::InvalidRequest,
                        error.to_string(),
                        false,
                    ),
                )
                .await;
                break;
            }
        };
        let frame: JsonlFrame = match serde_json::from_slice(&frame) {
            Ok(frame) => frame,
            Err(error) => {
                send_protocol_error(
                    &frame_tx,
                    "unknown",
                    "unknown",
                    ContractError::new(
                        ContractErrorCode::InvalidRequest,
                        format!("malformed JSONL frame: {error}"),
                        false,
                    ),
                )
                .await;
                continue;
            }
        };
        if let Err(error) = frame.validate_version() {
            let (frame_id, stream_id) = frame_ids(&frame);
            send_protocol_error(&frame_tx, &frame_id, &stream_id, error).await;
            continue;
        }
        match frame {
            JsonlFrame::Request {
                frame_id,
                stream_id,
                request,
                ..
            } => {
                if let Err(error) = validate_router_request(&request) {
                    send_protocol_error(
                        &frame_tx,
                        frame_id.as_str(),
                        stream_id.as_str(),
                        error,
                    )
                    .await;
                    continue;
                }
                let request_key = request.request_id.as_str().to_owned();
                if active.lock().await.contains_key(&request_key) {
                    send_protocol_error(
                        &frame_tx,
                        frame_id.as_str(),
                        stream_id.as_str(),
                        ContractError::new(
                            ContractErrorCode::InvalidRequest,
                            "request_id is already active on this connection",
                            false,
                        ),
                    )
                    .await;
                    continue;
                }
                let permit = match inflight.clone().try_acquire_owned() {
                    Ok(value) => value,
                    Err(_) => {
                        send_protocol_error(
                            &frame_tx,
                            frame_id.as_str(),
                            stream_id.as_str(),
                            ContractError::new(
                                ContractErrorCode::ProviderUnavailable,
                                "connection has too many active requests",
                                true,
                            ),
                        )
                        .await;
                        continue;
                    }
                };
                let cancellation = shutdown.child_token();
                active
                    .lock()
                    .await
                    .insert(request_key.clone(), cancellation.clone());
                let router = router.clone();
                let output = frame_tx.clone();
                let active_requests = active.clone();
                requests.spawn(async move {
                    let _permit = permit;
                    process_request(router, request, stream_id, output, cancellation).await;
                    active_requests.lock().await.remove(&request_key);
                });
            }
            JsonlFrame::Cancel {
                frame_id,
                stream_id,
                cancel,
                ..
            } => {
                let accepted = if let Some(token) =
                    active.lock().await.get(cancel.request_id.as_str())
                {
                    token.cancel();
                    true
                } else {
                    false
                };
                let ack = JsonlFrame::CancelAck {
                    version: PROTOCOL_VERSION_V1.to_owned(),
                    frame_id: next_id("frame"),
                    stream_id,
                    cancel_ack: CancelAck {
                        cancel_id: cancel.cancel_id,
                        request_id: cancel.request_id,
                        accepted,
                        final_status: if accepted {
                            RequestStatus::CancelRequested
                        } else {
                            RequestStatus::Failed
                        },
                        message: if accepted {
                            Some("cancellation forwarded to the active provider".to_owned())
                        } else {
                            Some("request was not active".to_owned())
                        },
                    },
                };
                let _ = frame_tx.send(ack).await;
                let _ = frame_id;
            }
            other => {
                let (frame_id, stream_id) = frame_ids(&other);
                send_protocol_error(
                    &frame_tx,
                    &frame_id,
                    &stream_id,
                    ContractError::new(
                        ContractErrorCode::InvalidRequest,
                        "router accepts only request and cancel frames from clients",
                        false,
                    ),
                )
                .await;
            }
        }
        while requests.try_join_next().is_some() {}
    }

    shutdown.cancel();
    for cancellation in active.lock().await.values() {
        cancellation.cancel();
    }
    while requests.join_next().await.is_some() {}
    drop(frame_tx);
    writer_task
        .await
        .map_err(|error| io::Error::new(io::ErrorKind::Other, error))?
}

async fn process_request(
    router: Arc<Router>,
    request: RouterRequest,
    stream_id: sentia_protocol::socket::StreamId,
    frames: mpsc::Sender<JsonlFrame>,
    cancellation: CancellationToken,
) {
    let request_id = request.request_id.as_str().to_owned();
    let (events, mut internal) = mpsc::channel(128);
    let decoded = match decode_request(&request) {
        Ok(decoded) => decoded,
        Err(error) => {
            let mut state =
                StreamState::new(request.request_id.clone(), stream_id);
            let _ = state
                .handle(
                    InternalEvent::Error {
                        request_id: request_id.clone(),
                        error,
                        failure: None,
                    },
                    &frames,
                )
                .await;
            let _ = state
                .handle(InternalEvent::Done { request_id }, &frames)
                .await;
            return;
        }
    };
    let router_task = match decoded {
        DecodedRequest::Control(control) => {
            let router = router.clone();
            tokio::spawn(async move {
                let event = match control {
                    ControlRequest::Status => router.status(&request_id).await,
                    ControlRequest::SettingsGet => InternalEvent::Control {
                        request_id: request_id.clone(),
                        response: ControlResponse::Settings {
                            settings: router.settings().get().await,
                        },
                    },
                    ControlRequest::SettingsUpdate { patch } => {
                        match router.settings().update(patch).await {
                            Ok(settings) => InternalEvent::Control {
                                request_id: request_id.clone(),
                                response: ControlResponse::Settings { settings },
                            },
                            Err(error) => error_event(
                                &request_id,
                                ContractErrorCode::Internal,
                                format!("could not save settings: {error}"),
                                false,
                                None,
                            ),
                        }
                    }
                    ControlRequest::ConsentPreview {
                        provider,
                        question,
                        context,
                        max_tokens,
                    } => {
                        router
                            .consent_preview(
                                &request_id,
                                &provider,
                                &question,
                                &context,
                                max_tokens,
                            )
                            .await
                    }
                };
                let _ = events.send(event).await;
                let _ = events
                    .send(InternalEvent::Done {
                        request_id: request_id.clone(),
                    })
                    .await;
            })
        }
        DecodedRequest::Chat(chat) => {
            let router = router.clone();
            tokio::spawn(async move {
                router.chat(chat, events, cancellation).await;
            })
        }
    };

    let mut state = StreamState::new(request.request_id.clone(), stream_id);
    while let Some(event) = internal.recv().await {
        if let Err(error) = state.handle(event, &frames).await {
            let _ = frames
                .send(error_frame(state.stream_id.clone(), error))
                .await;
            break;
        }
    }
    let _ = router_task.await;
}

enum DecodedRequest {
    Chat(ChatInput),
    Control(ControlRequest),
}

fn decode_request(request: &RouterRequest) -> Result<DecodedRequest, ContractError> {
    let mime = request.payload.mime_type.as_str();
    if request.capability == RouterCapability::Inference {
        let content = if mime == "text/plain" {
            ChatContent {
                question: request.payload.content.as_str().to_owned(),
                context: Vec::new(),
                max_tokens: None,
            }
        } else if mime == CHAT_MIME {
            serde_json::from_str(request.payload.content.as_str()).map_err(|error| {
                ContractError::new(
                    ContractErrorCode::InvalidRequest,
                    format!("invalid chat payload: {error}"),
                    false,
                )
            })?
        } else {
            return Err(ContractError::new(
                ContractErrorCode::InvalidRequest,
                "inference payload must be text/plain or the Sentia chat media type",
                false,
            ));
        };
        Ok(DecodedRequest::Chat(ChatInput {
            request_id: request.request_id.as_str().to_owned(),
            session_id: request.session_id.as_str().to_owned(),
            question: content.question,
            policy: request.policy,
            provider: request
                .provider_id
                .as_ref()
                .map(|value| value.as_str().to_owned()),
            context: content.context,
            consent_token: request.consent_token.clone(),
            max_tokens: content.max_tokens,
            provenance: request.provenance.clone(),
        }))
    } else if mime == CONTROL_MIME {
        let control =
            serde_json::from_str(request.payload.content.as_str()).map_err(|error| {
                ContractError::new(
                    ContractErrorCode::InvalidRequest,
                    format!("invalid router control payload: {error}"),
                    false,
                )
            })?;
        Ok(DecodedRequest::Control(control))
    } else {
        Err(ContractError::new(
            ContractErrorCode::InvalidRequest,
            "unsupported router capability or payload media type",
            false,
        ))
    }
}

fn validate_router_request(request: &RouterRequest) -> Result<(), ContractError> {
    request.validate()?;
    if request.request_id.as_str().is_empty() || request.session_id.as_str().is_empty() {
        return Err(ContractError::new(
            ContractErrorCode::ValidationFailed,
            "request_id and session_id must not be empty",
            false,
        ));
    }
    if request.payload.truncated {
        return Err(ContractError::new(
            ContractErrorCode::ValidationFailed,
            "router does not accept truncated input payloads",
            false,
        ));
    }
    Ok(())
}

struct StreamState {
    request_id: sentia_protocol::router::RequestId,
    stream_id: sentia_protocol::socket::StreamId,
    sequence: u64,
    cumulative_bytes: usize,
    current_answer: String,
    control: Option<ControlResponse>,
    error: Option<ContractError>,
    cancelled: bool,
    completed: bool,
    current_status: RequestStatus,
    current_provider: String,
}

impl StreamState {
    fn new(
        request_id: sentia_protocol::router::RequestId,
        stream_id: sentia_protocol::socket::StreamId,
    ) -> Self {
        Self {
            request_id,
            stream_id,
            sequence: 0,
            cumulative_bytes: 0,
            current_answer: String::new(),
            control: None,
            error: None,
            cancelled: false,
            completed: false,
            current_status: RequestStatus::Received,
            current_provider: "local".to_owned(),
        }
    }

    async fn handle(
        &mut self,
        event: InternalEvent,
        frames: &mpsc::Sender<JsonlFrame>,
    ) -> Result<(), ContractError> {
        match event {
            InternalEvent::Accepted { .. } => {}
            InternalEvent::StateTransition { from, to, .. } => {
                if !is_valid_status_transition(from, to) {
                    return Err(ContractError::new(
                        ContractErrorCode::Internal,
                        "router attempted an invalid status transition",
                        false,
                    ));
                }
                self.current_status = to;
                self.send_event(
                    frames,
                    to,
                    EventPayload::StateTransition { from, to },
                )
                .await;
            }
            InternalEvent::Progress { state, detail, .. } => {
                let health = if state == "starting" || state == "model_loading" {
                    ProviderHealth::Starting
                } else {
                    ProviderHealth::Degraded
                };
                let provider_state = if health == ProviderHealth::Starting {
                    ProviderState::Warming
                } else {
                    ProviderState::Backoff
                };
                let provider_status = status_with_message(
                    &self.current_provider,
                    health,
                    provider_state,
                    detail,
                );
                self.send_event(
                    frames,
                    self.current_status,
                    EventPayload::ProviderStatus { provider_status },
                )
                .await;
            }
            InternalEvent::AnswerStarted {
                provider_status,
                fallback_reason,
                ..
            } => {
                if fallback_reason.is_some() {
                    self.current_answer.clear();
                }
                self.current_status = if provider_status.provider_id.as_str() == "local" {
                    RequestStatus::RunningLocal
                } else {
                    RequestStatus::RunningRemote
                };
                self.current_provider = provider_status.provider_id.as_str().to_owned();
                self.send_event(
                    frames,
                    self.current_status,
                    EventPayload::ProviderStatus { provider_status },
                )
                .await;
            }
            InternalEvent::Delta { content, .. } => {
                self.current_answer.push_str(&content);
                for delta in split_bounded(&content, 8192) {
                    self.cumulative_bytes =
                        self.cumulative_bytes.saturating_add(delta.len());
                    self.send_event(
                        frames,
                        self.current_status,
                        EventPayload::OutputDelta {
                            delta: bounded(delta)?,
                            cumulative_bytes: self.cumulative_bytes.min(u32::MAX as usize) as u32,
                        },
                    )
                    .await;
                }
            }
            InternalEvent::ToolResult {
                tool, provenance, ..
            } => {
                if let Some(tool) = parse_tool_name(&tool) {
                    self.send_event(
                        frames,
                        self.current_status,
                        EventPayload::ToolProgress {
                            tool,
                            phase: ToolPhase::Completed,
                            detail: Some(provenance),
                        },
                    )
                    .await;
                }
            }
            InternalEvent::ActionProposal { tool, notice, .. } => {
                if let Some(tool) = parse_tool_name(&tool) {
                    self.send_event(
                        frames,
                        self.current_status,
                        EventPayload::ToolProgress {
                            tool,
                            phase: ToolPhase::Failed,
                            detail: Some(notice),
                        },
                    )
                    .await;
                }
            }
            InternalEvent::AnswerFinished { incomplete, .. } => {
                if !incomplete {
                    self.completed = true;
                }
            }
            InternalEvent::Control { response, .. } => {
                self.control = Some(response);
                self.completed = true;
            }
            InternalEvent::Cancelled { .. } => {
                self.cancelled = true;
            }
            InternalEvent::Error { error, .. } => {
                self.cancelled = error.code == ContractErrorCode::Cancelled;
                self.error = Some(error.clone());
                let _ = frames.send(error_frame(self.stream_id.clone(), error)).await;
            }
            InternalEvent::Done { .. } => {
                let status = if self.cancelled {
                    RequestStatus::Cancelled
                } else if self.error.is_some() || !self.completed {
                    RequestStatus::Failed
                } else {
                    RequestStatus::Completed
                };
                self.send_event(
                    frames,
                    status,
                    EventPayload::Terminal {
                        final_status: status,
                    },
                )
                .await;
                let result = self.result(status)?;
                let _ = frames
                    .send(JsonlFrame::Result {
                        version: PROTOCOL_VERSION_V1.to_owned(),
                        frame_id: next_id("frame"),
                        stream_id: self.stream_id.clone(),
                        result,
                    })
                    .await;
            }
        }
        Ok(())
    }

    async fn send_event(
        &mut self,
        frames: &mpsc::Sender<JsonlFrame>,
        status: RequestStatus,
        payload: EventPayload,
    ) {
        self.sequence = self.sequence.saturating_add(1);
        let frame = JsonlFrame::Event {
            version: PROTOCOL_VERSION_V1.to_owned(),
            frame_id: next_id("frame"),
            stream_id: self.stream_id.clone(),
            event: StreamEvent {
                event_id: next_id("event"),
                request_id: self.request_id.clone(),
                sequence: self.sequence,
                status,
                emitted_at_ms: unix_ms(),
                payload,
            },
        };
        let _ = frames.send(frame).await;
    }

    fn result(&self, status: RequestStatus) -> Result<RouterResult, ContractError> {
        let (payload, error) = if status == RequestStatus::Completed {
            let (mime, content) = if let Some(control) = &self.control {
                let mime = match control {
                    ControlResponse::Status { .. } => STATUS_MIME,
                    ControlResponse::Settings { .. } => SETTINGS_MIME,
                    ControlResponse::ConsentPreview { .. } => CONSENT_MIME,
                };
                (
                    mime,
                    serde_json::to_string(control).map_err(|error| {
                        ContractError::new(
                            ContractErrorCode::Internal,
                            format!("could not encode router response: {error}"),
                            false,
                        )
                    })?,
                )
            } else {
                ("text/plain", self.current_answer.clone())
            };
            let (content, truncated) = truncate_utf8(content, 262_144);
            (
                Some(ResultPayload {
                    mime_type: bounded(mime.to_owned())?,
                    byte_count: content.len() as u32,
                    content: bounded(content)?,
                    truncated,
                }),
                None,
            )
        } else {
            (None, self.error.clone())
        };
        let result = RouterResult {
            version: bounded(PROTOCOL_VERSION_V1.to_owned())?,
            request_id: self.request_id.clone(),
            status,
            provider_status: None,
            payload,
            error,
            metrics: Vec::new(),
        };
        result.validate()?;
        Ok(result)
    }
}

fn error_frame(
    stream_id: sentia_protocol::socket::StreamId,
    error: ContractError,
) -> JsonlFrame {
    JsonlFrame::Error {
        version: PROTOCOL_VERSION_V1.to_owned(),
        frame_id: next_id("frame"),
        stream_id,
        error,
    }
}

async fn send_protocol_error(
    frames: &mpsc::Sender<JsonlFrame>,
    frame_id: &str,
    stream_id: &str,
    error: ContractError,
) {
    let frame = JsonlFrame::Error {
        version: PROTOCOL_VERSION_V1.to_owned(),
        frame_id: BoundedString::new(frame_id).unwrap_or_else(|_| next_id("frame")),
        stream_id: BoundedString::new(stream_id).unwrap_or_else(|_| next_id("stream")),
        error,
    };
    let _ = frames.send(frame).await;
}

fn frame_ids(frame: &JsonlFrame) -> (String, String) {
    match frame {
        JsonlFrame::Request {
            frame_id,
            stream_id,
            ..
        }
        | JsonlFrame::Event {
            frame_id,
            stream_id,
            ..
        }
        | JsonlFrame::Result {
            frame_id,
            stream_id,
            ..
        }
        | JsonlFrame::Error {
            frame_id,
            stream_id,
            ..
        }
        | JsonlFrame::Cancel {
            frame_id,
            stream_id,
            ..
        }
        | JsonlFrame::CancelAck {
            frame_id,
            stream_id,
            ..
        }
        | JsonlFrame::ToolRequest {
            frame_id,
            stream_id,
            ..
        }
        | JsonlFrame::ToolResult {
            frame_id,
            stream_id,
            ..
        } => (frame_id.as_str().to_owned(), stream_id.as_str().to_owned()),
    }
}

fn status_with_message(
    provider: &str,
    health: ProviderHealth,
    state: ProviderState,
    message: String,
) -> ProviderStatus {
    ProviderStatus {
        provider_id: BoundedString::new(provider)
            .expect("built-in provider identifier is within bounds"),
        health,
        state,
        checked_at_ms: unix_ms(),
        message: Some(message),
        observed_latency_ms: None,
    }
}

fn parse_tool_name(name: &str) -> Option<ToolName> {
    serde_json::from_value(serde_json::Value::String(name.to_owned())).ok()
}

fn split_bounded(value: &str, maximum: usize) -> Vec<String> {
    if value.len() <= maximum {
        return vec![value.to_owned()];
    }
    let mut output = Vec::new();
    let mut start = 0;
    while start < value.len() {
        let mut end = (start + maximum).min(value.len());
        while end > start && !value.is_char_boundary(end) {
            end -= 1;
        }
        output.push(value[start..end].to_owned());
        start = end;
    }
    output
}

fn truncate_utf8(mut value: String, maximum: usize) -> (String, bool) {
    if value.len() <= maximum {
        return (value, false);
    }
    let mut end = maximum;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    (value, true)
}

fn bounded<const N: usize>(value: String) -> Result<BoundedString<N>, ContractError> {
    BoundedString::new(value).map_err(|error| {
        ContractError::new(
            ContractErrorCode::PayloadTooLarge,
            error.to_string(),
            false,
        )
    })
}

fn next_id(prefix: &str) -> BoundedString<64> {
    BoundedString::new(format!(
        "{prefix}-{}-{}",
        std::process::id(),
        FRAME_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ))
    .expect("generated identifier is within bounds")
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
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
                    "unterminated JSONL frame",
                ))
            };
        }
        let take = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| index + 1)
            .unwrap_or(available.len());
        if output.len().saturating_add(take) > maximum.min(MAX_FRAME_BYTES) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "JSONL frame exceeds configured size limit",
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

fn validate_peer(stream: &UnixStream) -> io::Result<()> {
    let credentials = stream.peer_cred()?;
    let expected = unsafe { libc::geteuid() };
    if credentials.uid() != expected {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "router socket peer UID does not match router UID",
        ));
    }
    Ok(())
}

// The fallback must never run when systemd already supplied the listener.
// bind_runtime_socket unlinks whatever is at the socket path and binds its own,
// so evaluating it eagerly replaced systemd's live socket file with one that was
// immediately dropped and closed. The path was left pointing at a dead socket
// and every client got ECONNREFUSED while both units reported active.
fn acquire_listener<S, B>(from_systemd: S, bind: B) -> io::Result<UnixListener>
where
    S: FnOnce() -> io::Result<Option<UnixListener>>,
    B: FnOnce() -> io::Result<UnixListener>,
{
    match from_systemd()? {
        Some(listener) => Ok(listener),
        None => bind(),
    }
}

fn bind_runtime_socket() -> io::Result<UnixListener> {
    let path = runtime_socket_path()?;
    bind_socket_path(&path)
}

fn bind_socket_path(path: &Path) -> io::Result<UnixListener> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "router socket has no parent")
    })?;
    ensure_private_directory(parent)?;
    if path.exists() {
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.file_type().is_socket() || metadata.uid() != unsafe { libc::geteuid() } {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "refusing to replace non-owned router socket path",
            ));
        }
        fs::remove_file(&path)?;
    }
    let listener = UnixListener::bind(&path)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

fn ensure_private_directory(path: &Path) -> io::Result<()> {
    if path.exists() {
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink()
            || !metadata.is_dir()
            || metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "router runtime directory is not a private owned directory",
            ));
        }
    } else {
        fs::create_dir_all(path)?;
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

fn listener_from_systemd() -> io::Result<Option<UnixListener>> {
    let listen_pid = env::var("LISTEN_PID")
        .ok()
        .and_then(|value| value.parse::<u32>().ok());
    let listen_fds = env::var("LISTEN_FDS")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(0);
    if listen_pid != Some(std::process::id()) || listen_fds == 0 {
        return Ok(None);
    }
    if listen_fds != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "sentia-router expects exactly one systemd socket",
        ));
    }
    let fd: RawFd = 3;
    let listener = unsafe { StdUnixListener::from_raw_fd(fd) };
    listener.set_nonblocking(true)?;
    let listener = UnixListener::from_std(listener)?;
    let address = listener.local_addr()?;
    let path = address.as_pathname().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "systemd supplied an unnamed router socket",
        )
    })?;
    let metadata = fs::metadata(path)?;
    if metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o777 != 0o600
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "systemd router socket ownership or mode is unsafe",
        ));
    }
    Ok(Some(listener))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn malformed_and_oversized_frames_are_bounded() {
        let mut valid = BufReader::new(&b"{\"version\":1}\nnext\n"[..]);
        assert_eq!(
            read_frame(&mut valid, 64).await.unwrap(),
            Some(b"{\"version\":1}".to_vec())
        );
        assert_eq!(
            read_frame(&mut valid, 64).await.unwrap(),
            Some(b"next".to_vec())
        );
        let mut oversized = BufReader::new(&b"123456\n"[..]);
        assert!(read_frame(&mut oversized, 4).await.is_err());
    }

    #[tokio::test]
    async fn router_socket_permissions_are_private() {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/test-state")
            .join(format!("router-socket-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        let socket = directory.join("sentia/router.sock");
        let _listener = bind_socket_path(&socket).unwrap();
        assert_eq!(
            fs::metadata(&socket).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::remove_file(&socket).unwrap();
        fs::remove_dir(directory.join("sentia")).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    // The socket path is left pointing at a dead socket if the bind fallback runs
    // while systemd has already supplied the listener, and every client then gets
    // ECONNREFUSED from units that both report active.
    #[tokio::test]
    async fn systemd_listener_suppresses_the_bind_fallback() {
        let directory = std::env::temp_dir()
            .join(format!("router-acquire-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        let socket = directory.join("sentia/router.sock");
        let supplied = bind_socket_path(&socket).unwrap();
        let supplied_inode = fs::symlink_metadata(&socket).unwrap().ino();

        let mut fallback_ran = false;
        let listener = acquire_listener(
            || Ok(Some(supplied)),
            || {
                fallback_ran = true;
                bind_socket_path(&socket)
            },
        )
        .unwrap();

        assert!(!fallback_ran, "bind fallback ran despite a systemd listener");
        assert_eq!(
            fs::symlink_metadata(&socket).unwrap().ino(),
            supplied_inode,
            "the socket file was replaced"
        );
        drop(listener);
        let _ = fs::remove_dir_all(&directory);
    }

    #[tokio::test]
    async fn bind_fallback_runs_without_a_systemd_listener() {
        let directory = std::env::temp_dir()
            .join(format!("router-acquire-none-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        let socket = directory.join("sentia/router.sock");

        let mut fallback_ran = false;
        let listener = acquire_listener(
            || Ok(None),
            || {
                fallback_ran = true;
                bind_socket_path(&socket)
            },
        )
        .unwrap();

        assert!(fallback_ran, "nothing bound the socket");
        assert!(fs::symlink_metadata(&socket).unwrap().file_type().is_socket());
        drop(listener);
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn utf8_chunks_respect_contract_bound() {
        let chunks = split_bounded(&"é".repeat(5000), 8192);
        assert!(chunks.iter().all(|chunk| chunk.len() <= 8192));
        assert_eq!(chunks.concat(), "é".repeat(5000));
    }
}
