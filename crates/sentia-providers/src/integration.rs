// SPDX-License-Identifier: Apache-2.0

use crate::{
    error::ProviderError,
    protocol::{
        InferenceRequest, ProviderCapabilities, ProviderId, WorkerCommand, WorkerEvent,
        PROTOCOL_VERSION, validate_request_id,
    },
};

/// Router-side integration contract for a provider worker transport.
///
/// Implementations frame only typed protocol commands. Process creation,
/// Secret Service access, consent, and privacy classification remain router
/// responsibilities and are intentionally absent from this trait.
pub trait ProviderWorkerIntegration: Send + Sync {
    fn provider(&self) -> ProviderId;

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::for_provider(self.provider())
    }

    fn inference_command(
        &self,
        request: InferenceRequest,
    ) -> Result<WorkerCommand, ProviderError>;

    fn cancellation_command(&self, request_id: String) -> Result<WorkerCommand, ProviderError>;

    fn encode_command(&self, command: &WorkerCommand) -> Result<String, ProviderError>;

    fn decode_event(&self, line: &str) -> Result<WorkerEvent, ProviderError>;
}

#[derive(Clone, Copy, Debug)]
pub struct JsonLineProviderIntegration {
    provider: ProviderId,
}

impl JsonLineProviderIntegration {
    pub fn new(provider: ProviderId) -> Self {
        Self { provider }
    }
}

impl ProviderWorkerIntegration for JsonLineProviderIntegration {
    fn provider(&self) -> ProviderId {
        self.provider
    }

    fn inference_command(
        &self,
        request: InferenceRequest,
    ) -> Result<WorkerCommand, ProviderError> {
        request.validate()?;
        if request.provider != self.provider {
            return Err(ProviderError::invalid(
                "provider_mismatch",
                "The request provider does not match this integration.",
            ));
        }
        Ok(WorkerCommand::Infer {
            protocol_version: PROTOCOL_VERSION,
            request,
        })
    }

    fn cancellation_command(&self, request_id: String) -> Result<WorkerCommand, ProviderError> {
        validate_request_id(&request_id)?;
        Ok(WorkerCommand::Cancel {
            protocol_version: PROTOCOL_VERSION,
            request_id,
        })
    }

    fn encode_command(&self, command: &WorkerCommand) -> Result<String, ProviderError> {
        if command.protocol_version() != PROTOCOL_VERSION {
            return Err(ProviderError::unsupported(
                "unsupported_protocol_version",
                "The worker protocol version is not supported.",
            ));
        }
        serde_json::to_string(command).map_err(|_| {
            ProviderError::process(
                "command_encoding_failed",
                "The provider command could not be encoded.",
            )
        })
    }

    fn decode_event(&self, line: &str) -> Result<WorkerEvent, ProviderError> {
        let event: WorkerEvent = serde_json::from_str(line).map_err(|_| {
            ProviderError::new(
                crate::error::ProviderErrorKind::MalformedResponse,
                "invalid_worker_event",
                "The provider worker returned invalid protocol JSON.",
                false,
            )
        })?;
        let version = match &event {
            WorkerEvent::Ready {
                protocol_version, ..
            }
            | WorkerEvent::Started {
                protocol_version, ..
            }
            | WorkerEvent::Completed {
                protocol_version, ..
            }
            | WorkerEvent::Error {
                protocol_version, ..
            } => *protocol_version,
        };
        if version != PROTOCOL_VERSION {
            return Err(ProviderError::unsupported(
                "unsupported_protocol_version",
                "The worker protocol version is not supported.",
            ));
        }
        Ok(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{Message, Role};

    #[test]
    fn protocol_round_trip_has_no_untyped_payload() {
        let integration = JsonLineProviderIntegration::new(ProviderId::OpenAi);
        let command = integration
            .inference_command(InferenceRequest {
                request_id: "request-1".to_owned(),
                provider: ProviderId::OpenAi,
                model: "configured-model".to_owned(),
                system: None,
                messages: vec![Message {
                    role: Role::User,
                    content: "hello".to_owned(),
                }],
                max_output_tokens: 32,
                temperature: None,
                timeout_ms: None,
                stream: false,
            })
            .unwrap();
        let encoded = integration.encode_command(&command).unwrap();
        assert!(!encoded.contains("\"tools\""));
        assert!(!encoded.contains("\"files\""));
        assert!(!encoded.contains("\"shell\""));
        assert!(!encoded.contains("\"endpoint\""));
    }
}
