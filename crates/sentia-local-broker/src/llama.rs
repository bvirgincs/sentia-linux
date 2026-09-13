use crate::config::BrokerConfig;
use sentia_local_broker::protocol::{
    BrokerEvent, BrokerMessage, BrokerTool, BrokerToolCall, BROKER_PROTOCOL_VERSION,
};
use sentia_protocol::{ProviderErrorCode, ProviderHealth};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    fs, io,
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    sync::Arc,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{UnixStream, unix::OwnedReadHalf},
    sync::mpsc,
    time::timeout,
};
use tokio_util::sync::CancellationToken;

const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_HEALTH_BODY_BYTES: usize = 256 * 1024;
const MAX_SSE_EVENT_BYTES: usize = 1024 * 1024;

#[derive(Clone)]
pub struct LlamaClient {
    config: Arc<BrokerConfig>,
}

#[derive(Debug)]
pub struct LlamaOutput {
    pub finish_reason: String,
    pub tool_calls: Vec<BrokerToolCall>,
}

#[derive(Clone, Debug, thiserror::Error)]
#[error("{message}")]
pub struct LlamaError {
    pub kind: ProviderErrorCode,
    pub message: String,
    pub retryable: bool,
}

impl LlamaClient {
    pub fn new(config: Arc<BrokerConfig>) -> Self {
        Self { config }
    }

    pub async fn health(&self) -> Result<(ProviderHealth, String), LlamaError> {
        let mut stream = self.connect().await?;
        stream
            .write_all(
                b"GET /health HTTP/1.1\r\nHost: sentia-local\r\nAccept: application/json\r\nConnection: close\r\n\r\n",
            )
            .await
            .map_err(io_error)?;
        let (read, _) = stream.into_split();
        let mut reader = BufReader::new(read);
        let head = read_head(&mut reader).await.map_err(io_error)?;
        let body = read_body_bounded(&mut reader, &head, MAX_HEALTH_BODY_BYTES)
            .await
            .map_err(io_error)?;
        let status_text = serde_json::from_slice::<Value>(&body)
            .ok()
            .and_then(|value| {
                value
                    .get("status")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| String::from_utf8_lossy(&body).trim().to_owned());
        if head.status == 200 {
            let state = if status_text.to_ascii_lowercase().contains("load") {
                ProviderHealth::Starting
            } else {
                ProviderHealth::Healthy
            };
            let detail = if state == ProviderHealth::Healthy {
                "local inference backend passed its health check".to_owned()
            } else {
                "local inference backend is loading the model".to_owned()
            };
            Ok((state, detail))
        } else if head.status == 503
            && status_text.to_ascii_lowercase().contains("load")
        {
            Ok((
                ProviderHealth::Starting,
                "local inference backend is loading the model".to_owned(),
            ))
        } else {
            Ok((
                ProviderHealth::Unavailable,
                format!("local inference backend health returned HTTP {}", head.status),
            ))
        }
    }

    pub async fn chat(
        &self,
        request_id: &str,
        messages: &[BrokerMessage],
        tools: &[BrokerTool],
        max_tokens: u32,
        events: &mpsc::Sender<BrokerEvent>,
        cancellation: CancellationToken,
    ) -> Result<LlamaOutput, LlamaError> {
        let body = build_chat_body(
            &self.config.backend_model,
            messages,
            tools,
            max_tokens,
        )?;
        let mut stream = tokio::select! {
            _ = cancellation.cancelled() => return Err(cancelled()),
            result = self.connect() => result?,
        };
        let header = format!(
            "POST /v1/chat/completions HTTP/1.1\r\nHost: sentia-local\r\nContent-Type: application/json\r\nAccept: text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        tokio::select! {
            _ = cancellation.cancelled() => return Err(cancelled()),
            result = async {
                stream.write_all(header.as_bytes()).await?;
                stream.write_all(&body).await
            } => result.map_err(io_error)?,
        }
        let (read, _) = stream.into_split();
        let mut reader = BufReader::new(read);
        let head = tokio::select! {
            _ = cancellation.cancelled() => return Err(cancelled()),
            result = read_head(&mut reader) => result.map_err(io_error)?,
        };
        if !(200..300).contains(&head.status) {
            let body = read_body_bounded(&mut reader, &head, MAX_HEALTH_BODY_BYTES)
                .await
                .map_err(io_error)?;
            return Err(http_error(head.status, &body));
        }
        let content_type = head
            .headers
            .get("content-type")
            .map(String::as_str)
            .unwrap_or_default();
        if content_type.contains("text/event-stream") || head.chunked {
            self.read_sse(request_id, reader, head, events, cancellation)
                .await
        } else {
            let body = tokio::select! {
                _ = cancellation.cancelled() => return Err(cancelled()),
                result = read_body_bounded(&mut reader, &head, 16 * 1024 * 1024) => result.map_err(io_error)?,
            };
            parse_non_stream_response(request_id, &body, events).await
        }
    }

    async fn connect(&self) -> Result<UnixStream, LlamaError> {
        let metadata = fs::symlink_metadata(&self.config.backend_socket).map_err(io_error)?;
        let parent = self.config.backend_socket.parent().ok_or_else(|| LlamaError {
            kind: ProviderErrorCode::ProcessFailure,
            message: "local inference socket path has no parent".to_owned(),
            retryable: false,
        })?;
        let parent_metadata = fs::symlink_metadata(parent).map_err(io_error)?;
        if !metadata.file_type().is_socket()
            || metadata.uid() == 0
            || metadata.permissions().mode() & 0o777 != 0o600
            || parent_metadata.uid() != metadata.uid()
            || parent_metadata.permissions().mode() & 0o777 != 0o700
        {
            return Err(LlamaError {
                kind: ProviderErrorCode::ProcessFailure,
                message: "local inference socket ownership or permissions are unsafe".to_owned(),
                retryable: false,
            });
        }
        let stream = timeout(
            self.config.connect_timeout(),
            UnixStream::connect(&self.config.backend_socket),
        )
        .await
        .map_err(|_| LlamaError {
            kind: ProviderErrorCode::Timeout,
            message: "timed out waiting for the local inference backend".to_owned(),
            retryable: true,
        })?
        .map_err(io_error)?;
        let peer = stream.peer_cred().map_err(io_error)?;
        if peer.uid() != metadata.uid() {
            return Err(LlamaError {
                kind: ProviderErrorCode::ProcessFailure,
                message: "local inference peer credentials did not match socket ownership"
                    .to_owned(),
                retryable: false,
            });
        }
        Ok(stream)
    }

    async fn read_sse(
        &self,
        request_id: &str,
        mut reader: BufReader<OwnedReadHalf>,
        head: HttpHead,
        events: &mpsc::Sender<BrokerEvent>,
        cancellation: CancellationToken,
    ) -> Result<LlamaOutput, LlamaError> {
        let mut body = BodyReader::new(head);
        let mut sse = Vec::new();
        let mut tool_calls = BTreeMap::<usize, ToolCallParts>::new();
        let mut finish_reason = None;
        // A reasoning model that spends its whole token budget thinking returns
        // a well-formed response whose content is empty, which would otherwise
        // reach the user as a blank answer with no explanation.
        let mut content_seen = false;
        loop {
            let chunk = tokio::select! {
                _ = cancellation.cancelled() => return Err(cancelled()),
                chunk = body.next(&mut reader) => chunk.map_err(io_error)?,
            };
            let Some(chunk) = chunk else {
                break;
            };
            sse.extend_from_slice(&chunk);
            if sse.len() > MAX_SSE_EVENT_BYTES {
                return Err(LlamaError {
                    kind: ProviderErrorCode::MalformedResponse,
                    message: "local inference SSE event exceeded size limit".to_owned(),
                    retryable: false,
                });
            }
            while let Some(event) = take_sse_event(&mut sse) {
                if process_sse_event(
                    request_id,
                    &event,
                    events,
                    &mut tool_calls,
                    &mut finish_reason,
                    &mut content_seen,
                )
                .await?
                {
                    let calls = finish_tool_calls(tool_calls)?;
                    ensure_answered(content_seen, &calls)?;
                    return Ok(LlamaOutput {
                        finish_reason: finish_reason.unwrap_or_else(|| "stop".to_owned()),
                        tool_calls: calls,
                    });
                }
            }
        }
        if !sse.is_empty() {
            let _ = process_sse_event(
                request_id,
                &sse,
                events,
                &mut tool_calls,
                &mut finish_reason,
                &mut content_seen,
            )
            .await?;
        }
        if finish_reason.is_none() {
            return Err(LlamaError {
                kind: ProviderErrorCode::ProcessFailure,
                message: "local inference stream ended before a terminal event".to_owned(),
                retryable: true,
            });
        }
        let calls = finish_tool_calls(tool_calls)?;
        ensure_answered(content_seen, &calls)?;
        Ok(LlamaOutput {
            finish_reason: finish_reason.expect("checked above"),
            tool_calls: calls,
        })
    }
}

/// A response that carries neither text nor a tool call is a failure, not an
/// empty answer. The most likely cause is a reasoning model configured to
/// think: its words land in `reasoning_content`, which is not an answer.
fn ensure_answered(content_seen: bool, tool_calls: &[BrokerToolCall]) -> Result<(), LlamaError> {
    if content_seen || !tool_calls.is_empty() {
        return Ok(());
    }
    Err(LlamaError {
        kind: ProviderErrorCode::MalformedResponse,
        message: "local inference produced no answer; if the model is a reasoning \
model, set SENTIA_REASONING=off or raise the token budget"
            .to_owned(),
        retryable: false,
    })
}

fn build_chat_body(
    model: &str,
    messages: &[BrokerMessage],
    tools: &[BrokerTool],
    max_tokens: u32,
) -> Result<Vec<u8>, LlamaError> {
    if model.contains(['\r', '\n']) {
        return Err(LlamaError {
            kind: ProviderErrorCode::MalformedResponse,
            message: "invalid configured backend model name".to_owned(),
            retryable: false,
        });
    }
    let messages = messages
        .iter()
        .map(|message| {
            let mut value = json!({
                "role": message.role,
                "content": message.content,
            });
            if let Some(name) = &message.name {
                value["name"] = json!(name);
            }
            if let Some(call_id) = &message.tool_call_id {
                value["tool_call_id"] = json!(call_id);
            }
            if !message.tool_calls.is_empty() {
                value["tool_calls"] = Value::Array(
                    message
                        .tool_calls
                        .iter()
                        .map(|call| {
                            json!({
                                "id": call.id,
                                "type": "function",
                                "function": {
                                    "name": call.name,
                                    "arguments": call.arguments,
                                }
                            })
                        })
                        .collect(),
                );
            }
            value
        })
        .collect::<Vec<_>>();
    let mut value = json!({
        "model": model,
        "messages": messages,
        "max_tokens": max_tokens,
        "stream": true,
    });
    if !tools.is_empty() {
        value["tools"] = Value::Array(
            tools
                .iter()
                .map(|tool| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.parameters,
                        }
                    })
                })
                .collect(),
        );
        value["tool_choice"] = json!("auto");
    }
    serde_json::to_vec(&value).map_err(|error| LlamaError {
        kind: ProviderErrorCode::MalformedResponse,
        message: format!("could not encode local inference request: {error}"),
        retryable: false,
    })
}

#[derive(Debug)]
struct HttpHead {
    status: u16,
    headers: HashMap<String, String>,
    chunked: bool,
    content_length: Option<usize>,
}

async fn read_head(reader: &mut BufReader<OwnedReadHalf>) -> io::Result<HttpHead> {
    let mut consumed = 0;
    let status_line = read_http_line(reader, &mut consumed).await?;
    let mut parts = status_line.split_whitespace();
    let version = parts.next().unwrap_or_default();
    let status = parts
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP status line"))?;
    if !version.starts_with("HTTP/1.") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported backend HTTP version",
        ));
    }
    let mut headers = HashMap::new();
    loop {
        let line = read_http_line(reader, &mut consumed).await?;
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':').ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "malformed backend HTTP header")
        })?;
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
    }
    let chunked = headers
        .get("transfer-encoding")
        .map(|value| value.to_ascii_lowercase().contains("chunked"))
        .unwrap_or(false);
    let content_length = headers
        .get("content-length")
        .map(|value| {
            value.parse::<usize>().map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP content-length")
            })
        })
        .transpose()?;
    Ok(HttpHead {
        status,
        headers,
        chunked,
        content_length,
    })
}

async fn read_http_line(
    reader: &mut BufReader<OwnedReadHalf>,
    consumed: &mut usize,
) -> io::Result<String> {
    let mut line = String::new();
    let count = reader.read_line(&mut line).await?;
    if count == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "backend closed during HTTP headers",
        ));
    }
    *consumed = consumed.saturating_add(count);
    if *consumed > MAX_HEADER_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "backend HTTP headers exceeded size limit",
        ));
    }
    Ok(line.trim_end_matches(['\r', '\n']).to_owned())
}

struct BodyReader {
    mode: BodyMode,
}

enum BodyMode {
    Chunked,
    Length(usize),
    UntilEof,
    Done,
}

impl BodyReader {
    fn new(head: HttpHead) -> Self {
        let mode = if head.chunked {
            BodyMode::Chunked
        } else if let Some(length) = head.content_length {
            BodyMode::Length(length)
        } else {
            BodyMode::UntilEof
        };
        Self { mode }
    }

    async fn next(
        &mut self,
        reader: &mut BufReader<OwnedReadHalf>,
    ) -> io::Result<Option<Vec<u8>>> {
        match self.mode {
            BodyMode::Chunked => {
                let mut line = String::new();
                let count = reader.read_line(&mut line).await?;
                if count == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "backend closed during chunk size",
                    ));
                }
                if line.len() > 128 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "invalid backend chunk size",
                    ));
                }
                let size_text = line
                    .trim()
                    .split(';')
                    .next()
                    .unwrap_or_default();
                let size = usize::from_str_radix(size_text, 16).map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "invalid backend chunk size")
                })?;
                if size == 0 {
                    loop {
                        line.clear();
                        if reader.read_line(&mut line).await? == 0 || line == "\r\n" || line == "\n" {
                            break;
                        }
                    }
                    self.mode = BodyMode::Done;
                    return Ok(None);
                }
                if size > MAX_SSE_EVENT_BYTES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "backend HTTP chunk exceeded size limit",
                    ));
                }
                let mut bytes = vec![0; size];
                reader.read_exact(&mut bytes).await?;
                let mut crlf = [0_u8; 2];
                reader.read_exact(&mut crlf).await?;
                if crlf != *b"\r\n" {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "malformed backend HTTP chunk",
                    ));
                }
                Ok(Some(bytes))
            }
            BodyMode::Length(remaining) => {
                if remaining == 0 {
                    self.mode = BodyMode::Done;
                    return Ok(None);
                }
                let amount = remaining.min(8192);
                let mut bytes = vec![0; amount];
                reader.read_exact(&mut bytes).await?;
                self.mode = BodyMode::Length(remaining - amount);
                Ok(Some(bytes))
            }
            BodyMode::UntilEof => {
                let mut bytes = vec![0; 8192];
                let count = reader.read(&mut bytes).await?;
                if count == 0 {
                    self.mode = BodyMode::Done;
                    Ok(None)
                } else {
                    bytes.truncate(count);
                    Ok(Some(bytes))
                }
            }
            BodyMode::Done => Ok(None),
        }
    }
}

async fn read_body_bounded(
    reader: &mut BufReader<OwnedReadHalf>,
    head: &HttpHead,
    maximum: usize,
) -> io::Result<Vec<u8>> {
    let copied_head = HttpHead {
        status: head.status,
        headers: head.headers.clone(),
        chunked: head.chunked,
        content_length: head.content_length,
    };
    let mut body_reader = BodyReader::new(copied_head);
    let mut output = Vec::new();
    while let Some(chunk) = body_reader.next(reader).await? {
        if output.len().saturating_add(chunk.len()) > maximum {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "backend HTTP body exceeded size limit",
            ));
        }
        output.extend_from_slice(&chunk);
    }
    Ok(output)
}

#[derive(Default)]
struct ToolCallParts {
    id: String,
    name: String,
    arguments: String,
}

async fn process_sse_event(
    request_id: &str,
    event: &[u8],
    events: &mpsc::Sender<BrokerEvent>,
    tool_calls: &mut BTreeMap<usize, ToolCallParts>,
    finish_reason: &mut Option<String>,
    content_seen: &mut bool,
) -> Result<bool, LlamaError> {
    let text = std::str::from_utf8(event).map_err(|_| LlamaError {
        kind: ProviderErrorCode::MalformedResponse,
        message: "local inference returned non-UTF-8 SSE".to_owned(),
        retryable: false,
    })?;
    let data = text
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim_start)
        .collect::<Vec<_>>()
        .join("\n");
    if data.is_empty() {
        return Ok(false);
    }
    if data.trim() == "[DONE]" {
        return Ok(true);
    }
    let value: Value = serde_json::from_str(&data).map_err(|_| LlamaError {
        kind: ProviderErrorCode::MalformedResponse,
        message: "local inference returned malformed SSE JSON".to_owned(),
        retryable: false,
    })?;
    if let Some(error) = value.get("error") {
        return Err(provider_json_error(error));
    }
    let choice = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .ok_or_else(|| LlamaError {
            kind: ProviderErrorCode::MalformedResponse,
            message: "local inference response omitted choices".to_owned(),
            retryable: false,
        })?;
    if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
        *finish_reason = Some(reason.to_owned());
    }
    let delta = choice.get("delta").unwrap_or(&Value::Null);
    if let Some(content) = delta.get("content").and_then(Value::as_str) {
        if !content.is_empty() {
            *content_seen = true;
            events
                .send(BrokerEvent::Delta {
                    version: BROKER_PROTOCOL_VERSION.to_owned(),
                    request_id: request_id.to_owned(),
                    content: content.to_owned(),
                })
                .await
                .map_err(|_| cancelled())?;
        }
    }
    if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            let index = call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            let parts = tool_calls.entry(index).or_default();
            if let Some(id) = call.get("id").and_then(Value::as_str) {
                parts.id.push_str(id);
            }
            if let Some(function) = call.get("function") {
                if let Some(name) = function.get("name").and_then(Value::as_str) {
                    parts.name.push_str(name);
                }
                if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
                    parts.arguments.push_str(arguments);
                }
            }
        }
    }
    Ok(false)
}

fn finish_tool_calls(
    values: BTreeMap<usize, ToolCallParts>,
) -> Result<Vec<BrokerToolCall>, LlamaError> {
    values
        .into_values()
        .map(|parts| {
            if parts.id.is_empty() || parts.name.is_empty() || parts.arguments.len() > 64 * 1024 {
                return Err(LlamaError {
                    kind: ProviderErrorCode::MalformedResponse,
                    message: "local inference returned an invalid tool call".to_owned(),
                    retryable: false,
                });
            }
            serde_json::from_str::<Value>(&parts.arguments).map_err(|_| LlamaError {
                kind: ProviderErrorCode::MalformedResponse,
                message: "local inference returned malformed tool arguments".to_owned(),
                retryable: false,
            })?;
            Ok(BrokerToolCall {
                id: parts.id,
                name: parts.name,
                arguments: parts.arguments,
            })
        })
        .collect()
}

async fn parse_non_stream_response(
    request_id: &str,
    body: &[u8],
    events: &mpsc::Sender<BrokerEvent>,
) -> Result<LlamaOutput, LlamaError> {
    let value: Value = serde_json::from_slice(body).map_err(|_| LlamaError {
        kind: ProviderErrorCode::MalformedResponse,
        message: "local inference returned malformed JSON".to_owned(),
        retryable: false,
    })?;
    if let Some(error) = value.get("error") {
        return Err(provider_json_error(error));
    }
    let choice = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|values| values.first())
        .ok_or_else(|| LlamaError {
            kind: ProviderErrorCode::MalformedResponse,
            message: "local inference response omitted choices".to_owned(),
            retryable: false,
        })?;
    let message = choice.get("message").unwrap_or(&Value::Null);
    let mut content_seen = false;
    if let Some(content) = message.get("content").and_then(Value::as_str) {
        if !content.is_empty() {
            content_seen = true;
            events
                .send(BrokerEvent::Delta {
                    version: BROKER_PROTOCOL_VERSION.to_owned(),
                    request_id: request_id.to_owned(),
                    content: content.to_owned(),
                })
                .await
                .map_err(|_| cancelled())?;
        }
    }
    let mut calls = Vec::new();
    if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in tool_calls {
            let id = call.get("id").and_then(Value::as_str).unwrap_or_default();
            let function = call.get("function").unwrap_or(&Value::Null);
            let name = function
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let arguments = function
                .get("arguments")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if id.is_empty()
                || name.is_empty()
                || arguments.len() > 64 * 1024
                || serde_json::from_str::<Value>(arguments).is_err()
            {
                return Err(LlamaError {
                    kind: ProviderErrorCode::MalformedResponse,
                    message: "local inference returned an invalid tool call".to_owned(),
                    retryable: false,
                });
            }
            calls.push(BrokerToolCall {
                id: id.to_owned(),
                name: name.to_owned(),
                arguments: arguments.to_owned(),
            });
        }
    }
    ensure_answered(content_seen, &calls)?;
    Ok(LlamaOutput {
        finish_reason: choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .unwrap_or("stop")
            .to_owned(),
        tool_calls: calls,
    })
}

fn take_sse_event(buffer: &mut Vec<u8>) -> Option<Vec<u8>> {
    let lf = buffer.windows(2).position(|window| window == b"\n\n");
    let crlf = buffer.windows(4).position(|window| window == b"\r\n\r\n");
    let (position, separator) = match (lf, crlf) {
        (Some(left), Some(right)) if left <= right => (left, 2),
        (Some(_), Some(right)) => (right, 4),
        (Some(left), None) => (left, 2),
        (None, Some(right)) => (right, 4),
        (None, None) => return None,
    };
    let event = buffer[..position].to_vec();
    buffer.drain(..position + separator);
    Some(event)
}

fn http_error(status: u16, body: &[u8]) -> LlamaError {
    let message = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| format!("local inference backend returned HTTP {status}"));
    let (kind, retryable) = match status {
        401 | 403 => (ProviderErrorCode::Authentication, false),
        408 | 504 => (ProviderErrorCode::Timeout, true),
        429 => (ProviderErrorCode::Quota, true),
        400 | 404 | 405 | 422 => (ProviderErrorCode::UnsupportedCapability, false),
        500..=599 => (ProviderErrorCode::ProcessFailure, true),
        _ => (ProviderErrorCode::MalformedResponse, false),
    };
    LlamaError {
        kind,
        message: bounded_error_message(message),
        retryable,
    }
}

fn provider_json_error(error: &Value) -> LlamaError {
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("local inference backend reported an error");
    LlamaError {
        kind: ProviderErrorCode::ProcessFailure,
        message: bounded_error_message(message.to_owned()),
        retryable: true,
    }
}

fn bounded_error_message(mut message: String) -> String {
    message.truncate(512);
    message
}

fn io_error(error: io::Error) -> LlamaError {
    LlamaError {
        kind: ProviderErrorCode::ProcessFailure,
        message: format!("local inference transport failed: {error}"),
        retryable: true,
    }
}

fn cancelled() -> LlamaError {
    LlamaError {
        kind: ProviderErrorCode::Internal,
        message: "local inference request cancelled".to_owned(),
        retryable: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tokio::net::UnixListener;

    #[test]
    fn parses_fragmented_sse_boundaries() {
        let mut buffer = b"data: {\"a\":1}\n\ndata: x\r\n\r\nremaining".to_vec();
        assert_eq!(
            take_sse_event(&mut buffer).unwrap(),
            b"data: {\"a\":1}".to_vec()
        );
        assert_eq!(take_sse_event(&mut buffer).unwrap(), b"data: x".to_vec());
        assert_eq!(buffer, b"remaining");
    }

    #[test]
    fn builds_tool_request_without_shelling_out() {
        let body = build_chat_body(
            "model",
            &[BrokerMessage {
                role: "user".to_owned(),
                content: "hello".to_owned(),
                name: None,
                tool_call_id: None,
                tool_calls: Vec::new(),
            }],
            &[BrokerTool {
                name: "health".to_owned(),
                description: "health".to_owned(),
                parameters: json!({"type":"object"}),
            }],
            128,
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["stream"], true);
        assert_eq!(value["tools"][0]["function"]["name"], "health");
    }

    #[tokio::test]
    async fn streams_openai_http_over_unix_socket() {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/test-state")
            .join(format!("llama-http-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let socket = directory.join("llama.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut byte = [0_u8; 1];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
            }
            let headers = String::from_utf8(request).unwrap();
            let length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|value| value.trim().parse::<usize>().ok())
                })
                .unwrap();
            let mut body = vec![0_u8; length];
            stream.read_exact(&mut body).await.unwrap();
            let body: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(body["stream"], true);
            let event =
                b"data: {\"choices\":[{\"delta\":{\"content\":\"hello\"},\"finish_reason\":\"stop\"}]}\n\n";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n",
                event.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            stream.write_all(event).await.unwrap();
            stream
                .write_all(b"\r\nE\r\ndata: [DONE]\n\n\r\n0\r\n\r\n")
                .await
                .unwrap();
        });
        let mut config = BrokerConfig::default();
        config.backend_socket = socket.clone();
        let client = LlamaClient::new(Arc::new(config));
        let (tx, mut rx) = mpsc::channel(8);
        let output = client
            .chat(
                "test-request",
                &[BrokerMessage {
                    role: "user".to_owned(),
                    content: "hello".to_owned(),
                    name: None,
                    tool_call_id: None,
                    tool_calls: Vec::new(),
                }],
                &[],
                32,
                &tx,
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(output.finish_reason, "stop");
        assert!(matches!(
            rx.recv().await,
            Some(BrokerEvent::Delta { content, .. }) if content == "hello"
        ));
        server.await.unwrap();
        fs::remove_file(socket).unwrap();
        fs::remove_dir(directory).unwrap();
    }
    #[tokio::test]
    async fn reasoning_only_response_is_an_error_not_a_blank_answer() {
        // Granite 4.2 thinks by default, and a truncated thought leaves
        // content empty while reasoning_content holds the text. Users must see
        // a reason rather than nothing at all.
        let body = br#"{"choices":[{"message":{"content":"","reasoning_content":"the user wants lsof"},"finish_reason":"length"}]}"#;
        let (tx, _rx) = mpsc::channel(8);
        let error = parse_non_stream_response("test-request", body, &tx)
            .await
            .expect_err("a response with no answer must fail");
        assert_eq!(error.kind, ProviderErrorCode::MalformedResponse);
        assert!(error.message.contains("no answer"), "{}", error.message);
    }
}
