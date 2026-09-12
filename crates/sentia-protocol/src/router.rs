use crate::bounded::{BoundedString, BoundedVec};
use crate::errors::{ContractError, ContractErrorCode};
use crate::metrics::MetricSample;
use crate::provider::{ProviderId, ProviderStatus};
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION_V1: &str = "sentia.v1";
pub const MAX_REQUEST_PAYLOAD_BYTES_V1: usize = 262_144;
pub const MAX_RESULT_PAYLOAD_BYTES_V1: usize = 262_144;

pub type ProtocolVersion = BoundedString<24>;
pub type RequestId = BoundedString<64>;
pub type SessionId = BoundedString<64>;
pub type TraceId = BoundedString<64>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RoutingPolicy {
    LocalOnly,
    LocalPreferred,
    RemotePreferred,
    AskBeforeRemote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouterCapability {
    Inference,
    Tooling,
    PackagePlanning,
    Health,
    Retrieval,
    IntentClassification,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceSource {
    UserInput,
    TerminalOutput,
    FileSnippet,
    SystemTool,
    ProviderResponse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataSensitivity {
    Public,
    Internal,
    Sensitive,
    Restricted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataProvenance {
    pub source: ProvenanceSource,
    pub sensitivity: DataSensitivity,
    pub redaction_applied: bool,
    pub origin_label: BoundedString<128>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<TraceId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestPayload {
    pub mime_type: BoundedString<64>,
    pub content: BoundedString<262_144>,
    pub byte_count: u32,
    pub truncated: bool,
}

impl RequestPayload {
    pub fn validate(&self) -> Result<(), ContractError> {
        let actual = self.content.as_str().as_bytes().len();
        if actual > MAX_REQUEST_PAYLOAD_BYTES_V1 {
            return Err(ContractError::new(
                ContractErrorCode::PayloadTooLarge,
                "request payload exceeds v1 byte bound",
                false,
            ));
        }
        if actual != self.byte_count as usize {
            return Err(ContractError::new(
                ContractErrorCode::ValidationFailed,
                "request payload byte_count does not match content length",
                false,
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResultPayload {
    pub mime_type: BoundedString<64>,
    pub content: BoundedString<262_144>,
    pub byte_count: u32,
    pub truncated: bool,
}

impl ResultPayload {
    pub fn validate(&self) -> Result<(), ContractError> {
        let actual = self.content.as_str().as_bytes().len();
        if actual > MAX_RESULT_PAYLOAD_BYTES_V1 {
            return Err(ContractError::new(
                ContractErrorCode::PayloadTooLarge,
                "result payload exceeds v1 byte bound",
                false,
            ));
        }
        if actual != self.byte_count as usize {
            return Err(ContractError::new(
                ContractErrorCode::ValidationFailed,
                "result payload byte_count does not match content length",
                false,
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsentToken {
    pub token_id: BoundedString<64>,
    pub payload_sha256: BoundedString<64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<ProviderId>,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouterRequest {
    pub version: ProtocolVersion,
    pub request_id: RequestId,
    pub session_id: SessionId,
    pub created_at_ms: u64,
    pub capability: RouterCapability,
    pub policy: RoutingPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<ProviderId>,
    pub provenance: DataProvenance,
    pub payload: RequestPayload,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consent_token: Option<ConsentToken>,
}

impl RouterRequest {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.version.as_str() != PROTOCOL_VERSION_V1 {
            return Err(ContractError::new(
                ContractErrorCode::UnsupportedVersion,
                "router request version is not sentia.v1",
                false,
            ));
        }
        self.payload.validate()?;
        if self.policy == RoutingPolicy::AskBeforeRemote && self.consent_token.is_none() {
            return Err(ContractError::new(
                ContractErrorCode::PolicyDenied,
                "ASK_BEFORE_REMOTE requires a consent token issued by trusted UI",
                false,
            ));
        }
        if let Some(token) = &self.consent_token {
            if token.expires_at_ms <= token.issued_at_ms {
                return Err(ContractError::new(
                    ContractErrorCode::ValidationFailed,
                    "consent token expiry must be after issue timestamp",
                    false,
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestStatus {
    Received,
    Queued,
    AwaitingConsent,
    RunningLocal,
    RunningRemote,
    FallbackToLocal,
    Completed,
    Failed,
    CancelRequested,
    Cancelled,
}

pub fn is_valid_status_transition(from: RequestStatus, to: RequestStatus) -> bool {
    use RequestStatus::*;
    matches!(
        (from, to),
        (Received, Queued)
            | (Received, AwaitingConsent)
            | (Queued, RunningLocal)
            | (Queued, RunningRemote)
            | (Queued, CancelRequested)
            | (AwaitingConsent, RunningRemote)
            | (AwaitingConsent, CancelRequested)
            | (RunningRemote, FallbackToLocal)
            | (RunningRemote, Completed)
            | (RunningRemote, Failed)
            | (RunningRemote, CancelRequested)
            | (FallbackToLocal, RunningLocal)
            | (RunningLocal, Completed)
            | (RunningLocal, Failed)
            | (RunningLocal, CancelRequested)
            | (CancelRequested, Cancelled)
    )
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouterResult {
    pub version: ProtocolVersion,
    pub request_id: RequestId,
    pub status: RequestStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_status: Option<ProviderStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<ResultPayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ContractError>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metrics: Vec<MetricSample>,
}

impl RouterResult {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.version.as_str() != PROTOCOL_VERSION_V1 {
            return Err(ContractError::new(
                ContractErrorCode::UnsupportedVersion,
                "router result version is not sentia.v1",
                false,
            ));
        }
        if let Some(payload) = &self.payload {
            payload.validate()?;
        }
        if matches!(self.status, RequestStatus::Completed) && self.payload.is_none() {
            return Err(ContractError::new(
                ContractErrorCode::ValidationFailed,
                "completed result requires payload",
                false,
            ));
        }
        if matches!(self.status, RequestStatus::Failed) && self.error.is_none() {
            return Err(ContractError::new(
                ContractErrorCode::ValidationFailed,
                "failed result requires structured error",
                false,
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouterConfig {
    pub version: ProtocolVersion,
    pub socket_path: String,
    pub default_policy: RoutingPolicy,
    pub allow_remote_providers: bool,
    pub max_in_flight_requests: u16,
    pub max_queue_depth: u16,
    pub max_payload_bytes: u32,
    pub max_stream_event_bytes: u32,
    pub allowed_provider_ids: BoundedVec<ProviderId, 32>,
}

impl RouterConfig {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.version.as_str() != PROTOCOL_VERSION_V1 {
            return Err(ContractError::new(
                ContractErrorCode::UnsupportedVersion,
                "router config version is not sentia.v1",
                false,
            ));
        }
        if self.max_payload_bytes as usize > MAX_REQUEST_PAYLOAD_BYTES_V1 {
            return Err(ContractError::new(
                ContractErrorCode::ValidationFailed,
                "router max_payload_bytes exceeds protocol bound",
                false,
            ));
        }
        Ok(())
    }
}
