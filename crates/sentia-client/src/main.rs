use sentia_protocol::{
    BoundedString, CancelReason, CancelRequest, ContractErrorCode, DataProvenance, DataSensitivity,
    EventPayload, JsonlFrame, ProvenanceSource, RequestPayload, RouterCapability, RouterRequest,
    RoutingPolicy, PROTOCOL_VERSION_V1,
};
use sentia_protocol::router::ConsentToken;
use sentia_router::{
    api::{
        ChatContent, ControlRequest, ControlResponse, PrivacyCategory, SettingsPatch, UserSettings,
        CHAT_MIME, CONTROL_MIME, MAX_FRAME_BYTES,
    },
    config::runtime_socket_path,
};
use std::{
    env, fs,
    io::{self, Read, Write},
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
};

static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Default)]
enum OutputMode {
    #[default]
    Human,
    Json,
    Silent,
}

struct Options {
    output: OutputMode,
    command: Command,
}

enum Command {
    Chat {
        question: String,
        policy: Option<RoutingPolicy>,
        provider: Option<String>,
        consent_token: Option<ConsentToken>,
    },
    Status,
    SettingsGet,
    SettingsPolicy(RoutingPolicy),
    SettingsProvider(Option<String>),
    SettingsPrivacy(PrivacyCategory, bool),
    AskRemote {
        provider: String,
        question: String,
    },
    Help,
}

#[tokio::main]
async fn main() {
    let result = match parse_args(env::args().skip(1).collect()) {
        Ok(options) => run(options).await,
        Err(message) => Err(io::Error::new(io::ErrorKind::InvalidInput, message)),
    };
    if let Err(error) = result {
        eprintln!("ai: {error}");
        std::process::exit(2);
    }
}

async fn run(options: Options) -> io::Result<()> {
    match options.command {
        Command::Help => {
            print_help();
            Ok(())
        }
        Command::Status => {
            let control = ControlRequest::Status;
            exchange(
                make_control_request(RouterCapability::Health, control)?,
                options.output,
            )
            .await?;
            Ok(())
        }
        Command::SettingsGet => {
            exchange(
                make_control_request(
                    RouterCapability::IntentClassification,
                    ControlRequest::SettingsGet,
                )?,
                options.output,
            )
            .await?;
            Ok(())
        }
        Command::SettingsPolicy(policy) => {
            update_settings(
                SettingsPatch {
                    default_policy: Some(policy),
                    preferred_remote_provider: None,
                    remote_categories: None,
                },
                options.output,
            )
            .await
        }
        Command::SettingsProvider(provider) => {
            update_settings(
                SettingsPatch {
                    default_policy: None,
                    preferred_remote_provider: Some(provider),
                    remote_categories: None,
                },
                options.output,
            )
            .await
        }
        Command::SettingsPrivacy(category, remote) => {
            let settings = fetch_settings().await?;
            let mut categories = settings.remote_categories;
            if remote {
                categories.insert(category);
            } else {
                categories.remove(&category);
            }
            update_settings(
                SettingsPatch {
                    default_policy: None,
                    preferred_remote_provider: None,
                    remote_categories: Some(categories),
                },
                options.output,
            )
            .await
        }
        Command::Chat {
            question,
            policy,
            provider,
            consent_token,
        } => {
            let policy = match policy {
                Some(policy) => policy,
                None => fetch_settings().await?.default_policy,
            };
            exchange(
                make_chat_request(question, policy, provider, consent_token)?,
                options.output,
            )
            .await?;
            Ok(())
        }
        Command::AskRemote { provider, question } => {
            ask_remote(&provider, &question, options.output).await
        }
    }
}

async fn ask_remote(provider: &str, question: &str, output: OutputMode) -> io::Result<()> {
    if matches!(output, OutputMode::Json) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "ask-remote is interactive; machine clients should submit a consent_preview control request and use its exact token",
        ));
    }
    let preview_frames = exchange(
        make_control_request(
            RouterCapability::IntentClassification,
            ControlRequest::ConsentPreview {
                provider: provider.to_owned(),
                question: question.to_owned(),
                context: Vec::new(),
                max_tokens: None,
            },
        )?,
        OutputMode::Silent,
    )
    .await?;
    let preview = control_response(&preview_frames)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "router omitted preview"))?;
    let (token, payload, redactions) = match preview {
        ControlResponse::ConsentPreview {
            token,
            payload,
            redactions,
            ..
        } => (token, payload, redactions),
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "router returned the wrong control response",
            ))
        }
    };
    eprintln!(
        "Exact payload proposed for {provider}:\n{}",
        serde_json::to_string_pretty(&payload).unwrap_or_default()
    );
    if !redactions.is_empty() {
        eprintln!("Redactions applied: {}", redactions.join(", "));
    }
    eprint!("Send this payload once? Type 'yes' to continue: ");
    io::stderr().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    if answer.trim() != "yes" {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "remote request was not consented",
        ));
    }
    exchange(
        make_chat_request(
            question.to_owned(),
            RoutingPolicy::AskBeforeRemote,
            Some(provider.to_owned()),
            Some(token),
        )?,
        OutputMode::Human,
    )
    .await?;
    Ok(())
}

async fn update_settings(patch: SettingsPatch, output: OutputMode) -> io::Result<()> {
    exchange(
        make_control_request(
            RouterCapability::IntentClassification,
            ControlRequest::SettingsUpdate { patch },
        )?,
        output,
    )
    .await?;
    Ok(())
}

async fn fetch_settings() -> io::Result<UserSettings> {
    let frames = exchange(
        make_control_request(
            RouterCapability::IntentClassification,
            ControlRequest::SettingsGet,
        )?,
        OutputMode::Silent,
    )
    .await?;
    match control_response(&frames)? {
        Some(ControlResponse::Settings { settings }) => Ok(settings),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "router omitted settings",
        )),
    }
}

fn make_chat_request(
    question: String,
    policy: RoutingPolicy,
    provider: Option<String>,
    consent_token: Option<ConsentToken>,
) -> io::Result<JsonlFrame> {
    let content = serde_json::to_string(&ChatContent {
        question,
        context: Vec::new(),
        max_tokens: None,
    })
    .map_err(invalid_data)?;
    make_request(
        RouterCapability::Inference,
        policy,
        provider,
        consent_token,
        CHAT_MIME,
        content,
    )
}

fn make_control_request(
    capability: RouterCapability,
    control: ControlRequest,
) -> io::Result<JsonlFrame> {
    let content = serde_json::to_string(&control).map_err(invalid_data)?;
    make_request(
        capability,
        RoutingPolicy::LocalOnly,
        None,
        None,
        CONTROL_MIME,
        content,
    )
}

fn make_request(
    capability: RouterCapability,
    policy: RoutingPolicy,
    provider: Option<String>,
    consent_token: Option<ConsentToken>,
    mime_type: &str,
    content: String,
) -> io::Result<JsonlFrame> {
    let request_id = generated_id("request")?;
    let stream_id = generated_id("stream")?;
    let request = RouterRequest {
        version: bounded(PROTOCOL_VERSION_V1)?,
        request_id: request_id.clone(),
        session_id: generated_id("session")?,
        created_at_ms: unix_ms(),
        capability,
        policy,
        provider_id: provider.map(bounded).transpose()?,
        provenance: DataProvenance {
            source: ProvenanceSource::UserInput,
            sensitivity: DataSensitivity::Internal,
            redaction_applied: false,
            origin_label: bounded("ai-cli")?,
            trace_id: None,
        },
        payload: RequestPayload {
            mime_type: bounded(mime_type)?,
            byte_count: content.len() as u32,
            content: bounded(content)?,
            truncated: false,
        },
        consent_token,
    };
    request.validate().map_err(contract_error)?;
    Ok(JsonlFrame::Request {
        version: PROTOCOL_VERSION_V1.to_owned(),
        frame_id: generated_id("frame")?,
        stream_id,
        request,
    })
}

async fn exchange(request: JsonlFrame, output: OutputMode) -> io::Result<Vec<JsonlFrame>> {
    let (target_request_id, target_stream_id) = match &request {
        JsonlFrame::Request {
            request, stream_id, ..
        } => (request.request_id.clone(), stream_id.clone()),
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "client can only initiate request frames",
            ))
        }
    };
    let socket = runtime_socket_path()?;
    let stream = connect_router(&socket).await?;
    let credentials = stream.peer_cred()?;
    if credentials.uid() != unsafe { libc::geteuid() } {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "router peer UID does not match the current user",
        ));
    }
    let (read, mut write) = stream.into_split();
    write_json_line(&mut write, &request).await?;
    let mut reader = BufReader::new(read);
    let mut frames = Vec::new();
    let mut interrupted = false;
    let mut failed = None;
    loop {
        let frame = if interrupted {
            read_frame(&mut reader, MAX_FRAME_BYTES).await?
        } else {
            tokio::select! {
                result = read_frame(&mut reader, MAX_FRAME_BYTES) => result?,
                result = tokio::signal::ctrl_c() => {
                    result?;
                    let cancel = JsonlFrame::Cancel {
                        version: PROTOCOL_VERSION_V1.to_owned(),
                        frame_id: generated_id("frame")?,
                        stream_id: target_stream_id.clone(),
                        cancel: CancelRequest {
                            cancel_id: generated_id("cancel")?,
                            request_id: target_request_id.clone(),
                            reason: CancelReason::UserRequested,
                            requested_at_ms: unix_ms(),
                        },
                    };
                    write_json_line(&mut write, &cancel).await?;
                    interrupted = true;
                    continue;
                }
            }
        };
        let Some(frame) = frame else {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "router closed before completing the request",
            ));
        };
        let frame: JsonlFrame = serde_json::from_slice(&frame).map_err(invalid_data)?;
        frame.validate_version().map_err(contract_error)?;
        render_frame(&frame, output)?;
        if let JsonlFrame::Error {
            stream_id, error, ..
        } = &frame
        {
            if stream_id == &target_stream_id {
                failed = Some(error.message.clone());
            }
        }
        let done = matches!(
            &frame,
            JsonlFrame::Result {
                stream_id,
                result,
                ..
            } if stream_id == &target_stream_id && result.request_id == target_request_id
        );
        frames.push(frame);
        if done {
            if matches!(output, OutputMode::Human) {
                println!();
            }
            if let Some(message) = failed {
                return Err(io::Error::new(io::ErrorKind::Other, message));
            }
            return Ok(frames);
        }
    }
}

async fn write_json_line(
    writer: &mut tokio::net::unix::OwnedWriteHalf,
    frame: &JsonlFrame,
) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(frame).map_err(invalid_data)?;
    bytes.push(b'\n');
    writer.write_all(&bytes).await
}

fn render_frame(frame: &JsonlFrame, output: OutputMode) -> io::Result<()> {
    if matches!(output, OutputMode::Json) {
        println!("{}", serde_json::to_string(frame).map_err(invalid_data)?);
        return Ok(());
    }
    if matches!(output, OutputMode::Silent) {
        return Ok(());
    }
    match frame {
        JsonlFrame::Event { event, .. } => match &event.payload {
            EventPayload::OutputDelta { delta, .. } => {
                print!("{}", delta.as_str());
                io::stdout().flush()?;
            }
            EventPayload::ProviderStatus { provider_status } => {
                if let Some(message) = &provider_status.message {
                    eprintln!("{message}");
                }
            }
            EventPayload::ToolProgress {
                tool,
                phase,
                detail,
            } => {
                eprintln!(
                    "[tool {}: {:?}{}]",
                    tool.as_str(),
                    phase,
                    detail
                        .as_ref()
                        .map(|value| format!(": {value}"))
                        .unwrap_or_default()
                );
            }
            _ => {}
        },
        JsonlFrame::Result { result, .. } => {
            if let Some(payload) = &result.payload {
                if payload.mime_type.as_str() != "text/plain" {
                    render_control_payload(payload.content.as_str())?;
                }
            }
        }
        JsonlFrame::Error { error, .. } => eprintln!("error: {}", error.message),
        JsonlFrame::CancelAck { cancel_ack, .. } => {
            eprintln!(
                "cancellation {}",
                if cancel_ack.accepted {
                    "accepted"
                } else {
                    "not accepted"
                }
            );
        }
        _ => {}
    }
    Ok(())
}

fn render_control_payload(content: &str) -> io::Result<()> {
    let response: ControlResponse = serde_json::from_str(content).map_err(invalid_data)?;
    match response {
        ControlResponse::Status {
            display,
            local,
            remotes,
        } => {
            println!("{display}");
            println!(
                "{}: {:?} ({})",
                local.provider_id,
                local.health,
                local.message.unwrap_or_default()
            );
            for remote in remotes {
                println!(
                    "{}: {:?} ({})",
                    remote.provider_id,
                    remote.health,
                    remote.message.unwrap_or_default()
                );
            }
        }
        ControlResponse::Settings { settings } => {
            println!("{}", serde_json::to_string_pretty(&settings).map_err(invalid_data)?);
        }
        ControlResponse::ConsentPreview {
            provider,
            token,
            redactions,
            ..
        } => {
            println!("Remote consent preview for {provider}");
            println!("payload sha256: {}", token.payload_sha256);
            if !redactions.is_empty() {
                println!("redactions: {}", redactions.join(", "));
            }
        }
    }
    Ok(())
}

fn control_response(frames: &[JsonlFrame]) -> io::Result<Option<ControlResponse>> {
    for frame in frames.iter().rev() {
        if let JsonlFrame::Result { result, .. } = frame {
            if let Some(payload) = &result.payload {
                if payload.mime_type.as_str() != "text/plain" {
                    return serde_json::from_str(payload.content.as_str())
                        .map(Some)
                        .map_err(invalid_data);
                }
            }
        }
    }
    Ok(None)
}

// The per-user router is socket-activated and starts with the session, so a
// terminal opened immediately after login can beat it to the socket by a few
// seconds. A raw "Connection refused (os error 111)" is the wrong thing to show
// a user in that window, so wait briefly and then explain what is actually
// wrong.
const ROUTER_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const ROUTER_CONNECT_RETRY: Duration = Duration::from_millis(250);

async fn connect_router(path: &std::path::Path) -> io::Result<UnixStream> {
    let deadline = Instant::now() + ROUTER_CONNECT_TIMEOUT;
    loop {
        let attempt = match verify_socket(path) {
            Ok(()) => UnixStream::connect(path).await,
            Err(error) => Err(error),
        };
        let error = match attempt {
            Ok(stream) => return Ok(stream),
            Err(error) => error,
        };
        // A permission or ownership failure is a real fault, not a race, and
        // must surface immediately rather than after a 30 second wait.
        let starting = matches!(
            error.kind(),
            io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
        );
        if !starting {
            return Err(error);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionRefused,
                format!(
                    "local AI is not available: nothing is accepting connections on {} after {}s. \
                     The per-user router starts with your session; check it with \
                     `systemctl --user status sentia-router.socket sentia-router.service`.",
                    path.display(),
                    ROUTER_CONNECT_TIMEOUT.as_secs()
                ),
            ));
        }
        tokio::time::sleep(ROUTER_CONNECT_RETRY).await;
    }
}

fn verify_socket(path: &std::path::Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_socket()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o777 != 0o600
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "router socket must be an owned socket with mode 0600",
        ));
    }
    Ok(())
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
                    "unterminated router response",
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
                "router response exceeds size limit",
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

fn parse_args(mut args: Vec<String>) -> Result<Options, String> {
    let output = if let Some(position) = args.iter().position(|arg| arg == "--json") {
        args.remove(position);
        OutputMode::Json
    } else {
        OutputMode::Human
    };
    if args.is_empty() {
        let mut question = String::new();
        io::stdin()
            .read_to_string(&mut question)
            .map_err(|error| error.to_string())?;
        if question.trim().is_empty() {
            return Ok(Options {
                output,
                command: Command::Help,
            });
        }
        return Ok(Options {
            output,
            command: Command::Chat {
                question,
                policy: None,
                provider: None,
                consent_token: None,
            },
        });
    }
    match args[0].as_str() {
        "-h" | "--help" | "help" => Ok(Options {
            output,
            command: Command::Help,
        }),
        "status" if args.len() == 1 => Ok(Options {
            output,
            command: Command::Status,
        }),
        "settings" if args.len() == 1 => Ok(Options {
            output,
            command: Command::SettingsGet,
        }),
        "settings" if args.len() == 3 && args[1] == "policy" => Ok(Options {
            output,
            command: Command::SettingsPolicy(parse_policy(&args[2])?),
        }),
        "settings" if args.len() == 3 && args[1] == "provider" => Ok(Options {
            output,
            command: Command::SettingsProvider(if args[2] == "none" {
                None
            } else {
                Some(args[2].clone())
            }),
        }),
        "settings" if args.len() == 4 && args[1] == "privacy" => Ok(Options {
            output,
            command: Command::SettingsPrivacy(
                parse_category(&args[2])?,
                match args[3].as_str() {
                    "remote" => true,
                    "local" => false,
                    _ => return Err("privacy scope must be local or remote".to_owned()),
                },
            ),
        }),
        "ask-remote" if args.len() >= 3 => Ok(Options {
            output,
            command: Command::AskRemote {
                provider: args[1].clone(),
                question: args[2..].join(" "),
            },
        }),
        _ => {
            let mut policy = None;
            let mut provider = None;
            let mut consent_token = None;
            let mut question = Vec::new();
            let mut index = 0;
            while index < args.len() {
                match args[index].as_str() {
                    "--policy" => {
                        index += 1;
                        policy = Some(parse_policy(
                            args.get(index)
                                .ok_or_else(|| "--policy requires a value".to_owned())?,
                        )?);
                    }
                    "--provider" => {
                        index += 1;
                        provider = Some(
                            args.get(index)
                                .ok_or_else(|| "--provider requires a value".to_owned())?
                                .clone(),
                        );
                    }
                    "--consent-token" => {
                        return Err(
                            "consent tokens are structured; use ask-remote or the JSONL API"
                                .to_owned(),
                        );
                    }
                    value if value.starts_with('-') => {
                        return Err(format!("unknown option: {value}"));
                    }
                    value => question.push(value.to_owned()),
                }
                index += 1;
            }
            if question.is_empty() {
                return Err("a question is required".to_owned());
            }
            Ok(Options {
                output,
                command: Command::Chat {
                    question: question.join(" "),
                    policy,
                    provider,
                    consent_token: consent_token.take(),
                },
            })
        }
    }
}

fn parse_policy(value: &str) -> Result<RoutingPolicy, String> {
    match value.to_ascii_uppercase().as_str() {
        "LOCAL_ONLY" => Ok(RoutingPolicy::LocalOnly),
        "LOCAL_PREFERRED" => Ok(RoutingPolicy::LocalPreferred),
        "REMOTE_PREFERRED" => Ok(RoutingPolicy::RemotePreferred),
        "ASK_BEFORE_REMOTE" => Ok(RoutingPolicy::AskBeforeRemote),
        _ => Err("unknown policy".to_owned()),
    }
}

fn parse_category(value: &str) -> Result<PrivacyCategory, String> {
    match value {
        "user_prompt" => Ok(PrivacyCategory::UserPrompt),
        "file_content" => Ok(PrivacyCategory::FileContent),
        "command_history" => Ok(PrivacyCategory::CommandHistory),
        "hostname" => Ok(PrivacyCategory::Hostname),
        "ip_address" => Ok(PrivacyCategory::IpAddress),
        "process_name" => Ok(PrivacyCategory::ProcessName),
        "journal" => Ok(PrivacyCategory::Journal),
        "diagnostic" => Ok(PrivacyCategory::Diagnostic),
        _ => Err("unknown privacy category".to_owned()),
    }
}

fn generated_id(prefix: &str) -> io::Result<BoundedString<64>> {
    bounded(format!(
        "{prefix}-{}-{}-{}",
        std::process::id(),
        unix_ms(),
        REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ))
}

fn bounded<const N: usize>(value: impl Into<String>) -> io::Result<BoundedString<N>> {
    BoundedString::new(value).map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn invalid_data(error: impl std::error::Error + Send + Sync + 'static) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

fn contract_error(error: sentia_protocol::ContractError) -> io::Error {
    let kind = match error.code {
        ContractErrorCode::PermissionDenied | ContractErrorCode::PolicyDenied => {
            io::ErrorKind::PermissionDenied
        }
        ContractErrorCode::Timeout => io::ErrorKind::TimedOut,
        _ => io::ErrorKind::InvalidData,
    };
    io::Error::new(kind, error.message)
}

fn print_help() {
    println!(
        "Usage:
  ai <question>
  ai                         # read question from stdin
  ai --json <question>
  ai status [--json]
  ai settings [--json]
  ai settings policy LOCAL_ONLY|LOCAL_PREFERRED|REMOTE_PREFERRED|ASK_BEFORE_REMOTE
  ai settings provider <name|none>
  ai settings privacy <category> <local|remote>
  ai ask-remote <provider> <question>

Options for chat:
  --policy <policy>
  --provider <provider>"
    );
}
