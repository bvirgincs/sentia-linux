use crate::bounded::{BoundedString, BoundedVec};
use serde::{Deserialize, Serialize};

pub type ProviderId = BoundedString<48>;
pub type ProviderName = BoundedString<64>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderHealth {
    Healthy,
    Degraded,
    Unavailable,
    Disabled,
    Starting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderState {
    Idle,
    Warming,
    Serving,
    Backoff,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCapability {
    InferenceText,
    InferenceStructured,
    Streaming,
    Cancellation,
    ConsentPreview,
    Classification,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderCapabilities {
    pub provider_id: ProviderId,
    pub provider_name: ProviderName,
    pub supports_remote_egress: bool,
    pub requires_explicit_consent: bool,
    pub max_input_bytes: u32,
    pub max_output_bytes: u32,
    pub capabilities: BoundedVec<ProviderCapability, 16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderStatus {
    pub provider_id: ProviderId,
    pub health: ProviderHealth,
    pub state: ProviderState,
    pub checked_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_latency_ms: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderErrorCode {
    Network,
    Dns,
    Authentication,
    Quota,
    Timeout,
    UnsupportedCapability,
    MalformedResponse,
    ProcessFailure,
    CircuitOpen,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderError {
    pub code: ProviderErrorCode,
    pub provider_id: ProviderId,
    pub retryable: bool,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_status: Option<u16>,
}
