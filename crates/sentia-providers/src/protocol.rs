// SPDX-License-Identifier: Apache-2.0

use crate::error::ProviderError;
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr, time::Duration};

pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_COMMAND_BYTES: usize = 1_100_000;
pub const MAX_TEXT_BYTES: usize = 1_048_576;
pub const MAX_RESPONSE_BYTES: usize = 4 * 1_048_576;
pub const DEFAULT_TIMEOUT_MS: u64 = 60_000;
pub const MAX_TIMEOUT_MS: u64 = 300_000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ProviderId {
    #[serde(rename = "openai")]
    OpenAi,
    #[serde(rename = "anthropic")]
    Anthropic,
    #[serde(rename = "gemini")]
    Gemini,
    #[serde(rename = "xai")]
    Xai,
    #[serde(rename = "deepseek")]
    DeepSeek,
}

impl ProviderId {
    pub const ALL: [Self; 5] = [
        Self::OpenAi,
        Self::Anthropic,
        Self::Gemini,
        Self::Xai,
        Self::DeepSeek,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
            Self::Xai => "xai",
            Self::DeepSeek => "deepseek",
        }
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ProviderId {
    type Err = ProviderError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "openai" => Ok(Self::OpenAi),
            "anthropic" => Ok(Self::Anthropic),
            "gemini" => Ok(Self::Gemini),
            "xai" => Ok(Self::Xai),
            "deepseek" => Ok(Self::DeepSeek),
            _ => Err(ProviderError::invalid(
                "unknown_provider",
                "The provider name is not supported.",
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InferenceRequest {
    pub request_id: String,
    pub provider: ProviderId,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub messages: Vec<Message>,
    pub max_output_tokens: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub stream: bool,
}

impl InferenceRequest {
    pub fn validate(&self) -> Result<(), ProviderError> {
        validate_request_id(&self.request_id)?;

        if self.model.is_empty()
            || self.model.len() > 200
            || !self.model.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
            })
        {
            return Err(ProviderError::invalid(
                "invalid_model",
                "model must be a configured, non-empty provider model identifier.",
            ));
        }

        if self.messages.is_empty() || self.messages.len() > 128 {
            return Err(ProviderError::invalid(
                "invalid_messages",
                "messages must contain between 1 and 128 text messages.",
            ));
        }

        let mut total_text = self.system.as_ref().map_or(0, String::len);
        for message in &self.messages {
            if message.content.is_empty() || message.content.len() > 262_144 {
                return Err(ProviderError::invalid(
                    "invalid_message_content",
                    "Each message must contain 1-262144 bytes of text.",
                ));
            }
            total_text = total_text.saturating_add(message.content.len());
        }
        if total_text > MAX_TEXT_BYTES {
            return Err(ProviderError::invalid(
                "request_too_large",
                "The sanitized request text exceeds the worker limit.",
            ));
        }

        if !(1..=65_536).contains(&self.max_output_tokens) {
            return Err(ProviderError::invalid(
                "invalid_max_output_tokens",
                "max_output_tokens must be between 1 and 65536.",
            ));
        }

        if let Some(temperature) = self.temperature {
            if !temperature.is_finite() || !(0.0..=2.0).contains(&temperature) {
                return Err(ProviderError::invalid(
                    "invalid_temperature",
                    "temperature must be finite and between 0 and 2.",
                ));
            }
        }

        if let Some(timeout_ms) = self.timeout_ms {
            if !(100..=MAX_TIMEOUT_MS).contains(&timeout_ms) {
                return Err(ProviderError::invalid(
                    "invalid_timeout",
                    "timeout_ms must be between 100 and 300000.",
                ));
            }
        }

        if self.stream {
            return Err(ProviderError::unsupported(
                "streaming_not_enabled",
                "Remote streaming is not enabled in protocol version 1.",
            ));
        }

        Ok(())
    }

    pub fn timeout(&self) -> Duration {
        Duration::from_millis(self.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS))
    }
}

pub fn validate_request_id(request_id: &str) -> Result<(), ProviderError> {
    if request_id.is_empty()
        || request_id.len() > 128
        || !request_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        Err(ProviderError::invalid(
            "invalid_request_id",
            "request_id must be 1-128 safe ASCII characters.",
        ))
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    Refusal,
    Incomplete,
    Other,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TokenUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InferenceResponse {
    pub provider: ProviderId,
    pub requested_model: String,
    pub text: String,
    pub finish_reason: FinishReason,
    pub incomplete: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiBilling {
    SeparatelyBilledApiUsage,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsumerSubscription {
    NotAnApiBillingEntitlement,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderVerification {
    OfficialDocsAndOfflineFixturesOnly,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderCapabilities {
    pub provider: ProviderId,
    pub remote_egress: bool,
    pub requires_router_consent: bool,
    pub privacy_classification: bool,
    pub text_generation: bool,
    pub streaming: bool,
    pub cancellation: bool,
    pub tool_calls: bool,
    pub server_state_requested: bool,
    pub max_input_bytes: u32,
    pub max_response_bytes: u32,
    pub live_tested: bool,
    pub enabled_by_default: bool,
    pub billing: ApiBilling,
    pub consumer_subscription: ConsumerSubscription,
    pub verification: ProviderVerification,
}

impl ProviderCapabilities {
    pub fn for_provider(provider: ProviderId) -> Self {
        Self {
            provider,
            remote_egress: true,
            requires_router_consent: true,
            privacy_classification: false,
            text_generation: true,
            streaming: false,
            cancellation: true,
            tool_calls: false,
            server_state_requested: false,
            max_input_bytes: MAX_COMMAND_BYTES as u32,
            max_response_bytes: MAX_RESPONSE_BYTES as u32,
            live_tested: false,
            enabled_by_default: false,
            billing: ApiBilling::SeparatelyBilledApiUsage,
            consumer_subscription: ConsumerSubscription::NotAnApiBillingEntitlement,
            verification: ProviderVerification::OfficialDocsAndOfflineFixturesOnly,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkerCommand {
    Infer {
        protocol_version: u16,
        request: InferenceRequest,
    },
    Cancel {
        protocol_version: u16,
        request_id: String,
    },
}

impl WorkerCommand {
    pub fn protocol_version(&self) -> u16 {
        match self {
            Self::Infer {
                protocol_version, ..
            }
            | Self::Cancel {
                protocol_version, ..
            } => *protocol_version,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkerEvent {
    Ready {
        protocol_version: u16,
        capabilities: ProviderCapabilities,
    },
    Started {
        protocol_version: u16,
        request_id: String,
    },
    Completed {
        protocol_version: u16,
        request_id: String,
        response: InferenceResponse,
    },
    Error {
        protocol_version: u16,
        #[serde(skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        error: ProviderError,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ProviderErrorKind;

    fn valid_request() -> InferenceRequest {
        InferenceRequest {
            request_id: "req-1".to_owned(),
            provider: ProviderId::OpenAi,
            model: "configured-model".to_owned(),
            system: None,
            messages: vec![Message {
                role: Role::User,
                content: "hello".to_owned(),
            }],
            max_output_tokens: 64,
            temperature: Some(0.2),
            timeout_ms: Some(2_000),
            stream: false,
        }
    }

    #[test]
    fn rejects_endpoint_shaped_model_identifier() {
        let mut request = valid_request();
        request.model = "../../private".to_owned();
        assert_eq!(
            request.validate().unwrap_err().code,
            "invalid_model"
        );
    }

    #[test]
    fn rejects_streaming_until_sse_parsers_are_implemented() {
        let mut request = valid_request();
        request.stream = true;
        assert_eq!(
            request.validate().unwrap_err().kind,
            ProviderErrorKind::UnsupportedCapability
        );
    }

    #[test]
    fn capabilities_keep_privacy_and_billing_boundaries_explicit() {
        let capabilities = ProviderCapabilities::for_provider(ProviderId::OpenAi);
        assert!(capabilities.remote_egress);
        assert!(capabilities.requires_router_consent);
        assert!(!capabilities.privacy_classification);
        assert!(!capabilities.tool_calls);
        assert!(!capabilities.live_tested);
        assert!(!capabilities.enabled_by_default);
        assert_eq!(
            capabilities.billing,
            ApiBilling::SeparatelyBilledApiUsage
        );
        assert_eq!(
            capabilities.consumer_subscription,
            ConsumerSubscription::NotAnApiBillingEntitlement
        );
    }

    #[test]
    fn provider_ids_match_profile_names() {
        let encoded: Vec<String> = ProviderId::ALL
            .into_iter()
            .map(|provider| serde_json::to_string(&provider).unwrap())
            .collect();
        assert_eq!(
            encoded,
            ["\"openai\"", "\"anthropic\"", "\"gemini\"", "\"xai\"", "\"deepseek\""]
        );
    }
}
