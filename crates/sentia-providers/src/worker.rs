// SPDX-License-Identifier: Apache-2.0

use crate::{
    adapter,
    credential::SecretBytes,
    error::{ProviderError, ProviderErrorKind},
    protocol::{
        validate_request_id, ProviderCapabilities, ProviderId, WorkerCommand, WorkerEvent,
        MAX_COMMAND_BYTES, PROTOCOL_VERSION,
    },
    transport::ProviderTransport,
};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::task::JoinHandle;
use tokio_util::codec::{FramedRead, FramedWrite, LinesCodec};

struct ActiveRequest {
    request_id: String,
    task: JoinHandle<Result<crate::protocol::InferenceResponse, ProviderError>>,
}

pub async fn run_stdio(provider: ProviderId) -> Result<(), ProviderError> {
    crate::credential::harden_process()?;
    let credential = Arc::new(SecretBytes::read_inherited()?);
    let transport = ProviderTransport::new()?;
    let adapter = adapter::for_provider(provider);
    let mut input = FramedRead::new(
        tokio::io::stdin(),
        LinesCodec::new_with_max_length(MAX_COMMAND_BYTES),
    );
    let mut output = FramedWrite::new(tokio::io::stdout(), LinesCodec::new());

    write_event(
        &mut output,
        &WorkerEvent::Ready {
            protocol_version: PROTOCOL_VERSION,
            capabilities: ProviderCapabilities::for_provider(provider),
        },
    )
    .await?;

    let mut active: Option<ActiveRequest> = None;
    loop {
        if let Some(mut current) = active.take() {
            tokio::select! {
                result = &mut current.task => {
                    let event = match result {
                        Ok(Ok(response)) => WorkerEvent::Completed {
                            protocol_version: PROTOCOL_VERSION,
                            request_id: current.request_id,
                            response,
                        },
                        Ok(Err(error)) => WorkerEvent::Error {
                            protocol_version: PROTOCOL_VERSION,
                            request_id: Some(current.request_id),
                            error,
                        },
                        Err(_) => WorkerEvent::Error {
                            protocol_version: PROTOCOL_VERSION,
                            request_id: Some(current.request_id),
                            error: ProviderError::process(
                                "worker_task_failed",
                                "The provider request task failed.",
                            ),
                        },
                    };
                    write_event(&mut output, &event).await?;
                }
                line = input.next() => {
                    let Some(line) = line else {
                        current.task.abort();
                        return Ok(());
                    };
                    let command = match decode_command(line) {
                        Ok(command) => command,
                        Err(error) => {
                            write_error(&mut output, None, error).await?;
                            active = Some(current);
                            continue;
                        }
                    };
                    match command {
                        WorkerCommand::Cancel { request_id, .. }
                            if request_id == current.request_id =>
                        {
                            current.task.abort();
                            write_error(
                                &mut output,
                                Some(request_id),
                                ProviderError::new(
                                    ProviderErrorKind::Cancelled,
                                    "cancelled",
                                    "The provider request was cancelled.",
                                    false,
                                ),
                            )
                            .await?;
                        }
                        WorkerCommand::Cancel { request_id, .. } => {
                            write_error(
                                &mut output,
                                Some(request_id),
                                ProviderError::invalid(
                                    "request_not_active",
                                    "The cancellation request did not match the active request.",
                                ),
                            )
                            .await?;
                            active = Some(current);
                        }
                        WorkerCommand::Infer { request, .. } => {
                            let request_id = if validate_request_id(&request.request_id).is_ok() {
                                Some(request.request_id)
                            } else {
                                None
                            };
                            write_error(
                                &mut output,
                                request_id,
                                ProviderError::new(
                                    ProviderErrorKind::ProcessFailure,
                                    "worker_busy",
                                    "The provider worker already has an active request.",
                                    true,
                                ),
                            )
                            .await?;
                            active = Some(current);
                        }
                    }
                }
            }
        } else {
            let Some(line) = input.next().await else {
                return Ok(());
            };
            let command = match decode_command(line) {
                Ok(command) => command,
                Err(error) => {
                    write_error(&mut output, None, error).await?;
                    continue;
                }
            };
            match command {
                WorkerCommand::Cancel { request_id, .. } => {
                    write_error(
                        &mut output,
                        Some(request_id),
                        ProviderError::invalid(
                            "request_not_active",
                            "There is no active provider request to cancel.",
                        ),
                    )
                    .await?;
                }
                WorkerCommand::Infer { request, .. } => {
                    if let Err(error) = request.validate() {
                        let request_id = if validate_request_id(&request.request_id).is_ok() {
                            Some(request.request_id)
                        } else {
                            None
                        };
                        write_error(&mut output, request_id, error).await?;
                        continue;
                    }
                    if request.provider != provider {
                        write_error(
                            &mut output,
                            Some(request.request_id),
                            ProviderError::invalid(
                                "provider_mismatch",
                                "The request provider does not match this worker.",
                            ),
                        )
                        .await?;
                        continue;
                    }

                    let request_id = request.request_id.clone();
                    write_event(
                        &mut output,
                        &WorkerEvent::Started {
                            protocol_version: PROTOCOL_VERSION,
                            request_id: request_id.clone(),
                        },
                    )
                    .await?;
                    let transport = transport.clone();
                    let adapter = Arc::clone(&adapter);
                    let credential = Arc::clone(&credential);
                    active = Some(ActiveRequest {
                        request_id,
                        task: tokio::spawn(async move {
                            transport.infer(adapter, request, credential).await
                        }),
                    });
                }
            }
        }
    }
}

fn decode_command(
    line: Result<String, tokio_util::codec::LinesCodecError>,
) -> Result<WorkerCommand, ProviderError> {
    let line = line.map_err(|_| {
        ProviderError::invalid(
            "command_too_large",
            "The worker command exceeded the protocol frame limit.",
        )
    })?;
    let command: WorkerCommand = serde_json::from_str(&line).map_err(|_| {
        ProviderError::invalid(
            "invalid_command",
            "The worker command was not valid protocol JSON.",
        )
    })?;
    if command.protocol_version() != PROTOCOL_VERSION {
        return Err(ProviderError::unsupported(
            "unsupported_protocol_version",
            "The worker protocol version is not supported.",
        ));
    }
    if let WorkerCommand::Cancel { request_id, .. } = &command {
        validate_request_id(request_id)?;
    }
    Ok(command)
}

async fn write_error(
    output: &mut FramedWrite<tokio::io::Stdout, LinesCodec>,
    request_id: Option<String>,
    error: ProviderError,
) -> Result<(), ProviderError> {
    write_event(
        output,
        &WorkerEvent::Error {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            error,
        },
    )
    .await
}

async fn write_event(
    output: &mut FramedWrite<tokio::io::Stdout, LinesCodec>,
    event: &WorkerEvent,
) -> Result<(), ProviderError> {
    let line = serde_json::to_string(event).map_err(|_| {
        ProviderError::process(
            "response_encoding_failed",
            "The worker response could not be encoded.",
        )
    })?;
    output.send(line).await.map_err(|_| {
        ProviderError::process(
            "response_write_failed",
            "The worker response channel closed.",
        )
    })
}
