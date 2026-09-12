use crate::bounded::{BoundedString, BoundedVec};
use crate::errors::{ContractError, ContractErrorCode};
use crate::metrics::MetricSample;
use crate::provider::{ProviderId, ProviderStatus};
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION_V1: &str = "sentia.v1";
pub const MAX_REQUEST_PAYLOAD_BYTES_V1: usize = 262_144;
pub const MAX_RESULT_PAYLOAD_BYTES_V1: usize = 262_144;
pub const INFERENCE_SOCKET_PATH_V1: &str = "/run/sentia-inference/llama.sock";
pub const LOCAL_BROKER_SOCKET_PATH_V1: &str = "/run/sentia-local/broker.sock";
pub const HEALTH_METRICS_SOCKET_PATH_V1: &str = "/run/sentia-health/metrics.sock";
pub const USER_ROUTER_SOCKET_TEMPLATE_V1: &str = "$XDG_RUNTIME_DIR/sentia/router.sock";

pub type ProtocolVersion = BoundedString<24>;
pub type RequestId = BoundedString<64>;
pub type SessionId = BoundedString<64>;
pub type TraceId = BoundedString<64>;
pub type SocketPath = BoundedString<256>;
pub type SocketMode = BoundedString<4>;
pub type SystemUser = BoundedString<64>;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SocketEndpointRole {
    InferencePrivate,
    LocalBroker,
    HealthMetrics,
    UserRouter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SocketAccessScope {
    PrivateSystem,
    LocalMultiUser,
    PerUser,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SocketEndpointPolicy {
    pub role: SocketEndpointRole,
    pub access_scope: SocketAccessScope,
    pub path: SocketPath,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_user: Option<SystemUser>,
    pub directory_mode_octal: SocketMode,
    pub socket_mode_octal: SocketMode,
    pub require_peer_uid_check: bool,
    pub enforce_peer_uid_quotas: bool,
    pub nonsecret_metrics_only: bool,
    pub allow_cross_user_journal_access: bool,
    pub privileged_mutation_authority: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deviation_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntegrationSocketDefaults {
    pub inference: SocketEndpointPolicy,
    pub local_broker: SocketEndpointPolicy,
    pub health_metrics: SocketEndpointPolicy,
    pub user_router: SocketEndpointPolicy,
}

impl Default for IntegrationSocketDefaults {
    fn default() -> Self {
        Self {
            inference: SocketEndpointPolicy {
                role: SocketEndpointRole::InferencePrivate,
                access_scope: SocketAccessScope::PrivateSystem,
                path: bounded_socket_path(INFERENCE_SOCKET_PATH_V1),
                owner_user: Some(bounded_system_user("sentia-inference")),
                directory_mode_octal: bounded_socket_mode("0700"),
                socket_mode_octal: bounded_socket_mode("0600"),
                require_peer_uid_check: true,
                enforce_peer_uid_quotas: true,
                nonsecret_metrics_only: false,
                allow_cross_user_journal_access: false,
                privileged_mutation_authority: false,
                deviation_reason: None,
            },
            local_broker: SocketEndpointPolicy {
                role: SocketEndpointRole::LocalBroker,
                access_scope: SocketAccessScope::LocalMultiUser,
                path: bounded_socket_path(LOCAL_BROKER_SOCKET_PATH_V1),
                owner_user: None,
                directory_mode_octal: bounded_socket_mode("0755"),
                socket_mode_octal: bounded_socket_mode("0660"),
                require_peer_uid_check: true,
                enforce_peer_uid_quotas: true,
                nonsecret_metrics_only: false,
                allow_cross_user_journal_access: false,
                privileged_mutation_authority: false,
                deviation_reason: None,
            },
            health_metrics: SocketEndpointPolicy {
                role: SocketEndpointRole::HealthMetrics,
                access_scope: SocketAccessScope::LocalMultiUser,
                path: bounded_socket_path(HEALTH_METRICS_SOCKET_PATH_V1),
                owner_user: None,
                directory_mode_octal: bounded_socket_mode("0755"),
                socket_mode_octal: bounded_socket_mode("0660"),
                require_peer_uid_check: true,
                enforce_peer_uid_quotas: true,
                nonsecret_metrics_only: true,
                allow_cross_user_journal_access: false,
                privileged_mutation_authority: false,
                deviation_reason: None,
            },
            user_router: SocketEndpointPolicy {
                role: SocketEndpointRole::UserRouter,
                access_scope: SocketAccessScope::PerUser,
                path: bounded_socket_path(USER_ROUTER_SOCKET_TEMPLATE_V1),
                owner_user: None,
                directory_mode_octal: bounded_socket_mode("0700"),
                socket_mode_octal: bounded_socket_mode("0600"),
                require_peer_uid_check: true,
                enforce_peer_uid_quotas: true,
                nonsecret_metrics_only: false,
                allow_cross_user_journal_access: false,
                privileged_mutation_authority: false,
                deviation_reason: None,
            },
        }
    }
}

impl IntegrationSocketDefaults {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.inference.role != SocketEndpointRole::InferencePrivate
            || self.local_broker.role != SocketEndpointRole::LocalBroker
            || self.health_metrics.role != SocketEndpointRole::HealthMetrics
            || self.user_router.role != SocketEndpointRole::UserRouter
        {
            return Err(ContractError::new(
                ContractErrorCode::ValidationFailed,
                "integration socket roles do not match required endpoint slots",
                false,
            ));
        }
        self.validate_endpoint(&self.inference)?;
        self.validate_endpoint(&self.local_broker)?;
        self.validate_endpoint(&self.health_metrics)?;
        self.validate_endpoint(&self.user_router)?;
        Ok(())
    }

    pub fn deviations(&self) -> Vec<String> {
        [
            &self.inference,
            &self.local_broker,
            &self.health_metrics,
            &self.user_router,
        ]
        .into_iter()
        .filter_map(|endpoint| {
            endpoint
                .deviation_reason
                .as_ref()
                .map(|reason| format!("{:?}: {}", endpoint.role, reason))
        })
        .collect()
    }

    fn validate_endpoint(&self, endpoint: &SocketEndpointPolicy) -> Result<(), ContractError> {
        validate_socket_mode(endpoint.directory_mode_octal.as_str(), "directory_mode_octal")?;
        validate_socket_mode(endpoint.socket_mode_octal.as_str(), "socket_mode_octal")?;

        if endpoint.privileged_mutation_authority {
            return Err(ContractError::new(
                ContractErrorCode::ValidationFailed,
                "public metrics and inference sockets must not grant privileged mutation authority",
                false,
            ));
        }

        match endpoint.role {
            SocketEndpointRole::InferencePrivate => {
                if endpoint.access_scope != SocketAccessScope::PrivateSystem {
                    return Err(ContractError::new(
                        ContractErrorCode::ValidationFailed,
                        "inference endpoint must use private_system scope",
                        false,
                    ));
                }
                ensure_default_or_deviation(
                    endpoint.path.as_str(),
                    INFERENCE_SOCKET_PATH_V1,
                    endpoint.deviation_reason.as_ref(),
                    "inference socket path",
                )?;
                ensure_default_or_deviation(
                    endpoint.directory_mode_octal.as_str(),
                    "0700",
                    endpoint.deviation_reason.as_ref(),
                    "inference directory mode",
                )?;
                let owner = endpoint.owner_user.as_ref().map(|v| v.as_str()).unwrap_or("");
                ensure_default_or_deviation(
                    owner,
                    "sentia-inference",
                    endpoint.deviation_reason.as_ref(),
                    "inference owner user",
                )?;
                if !endpoint.require_peer_uid_check {
                    return Err(ContractError::new(
                        ContractErrorCode::ValidationFailed,
                        "inference socket must enforce peer uid checks",
                        false,
                    ));
                }
            }
            SocketEndpointRole::LocalBroker => {
                if endpoint.access_scope != SocketAccessScope::LocalMultiUser {
                    return Err(ContractError::new(
                        ContractErrorCode::ValidationFailed,
                        "local broker endpoint must use local_multi_user scope",
                        false,
                    ));
                }
                ensure_default_or_deviation(
                    endpoint.path.as_str(),
                    LOCAL_BROKER_SOCKET_PATH_V1,
                    endpoint.deviation_reason.as_ref(),
                    "local broker socket path",
                )?;
                if !endpoint.require_peer_uid_check || !endpoint.enforce_peer_uid_quotas {
                    return Err(ContractError::new(
                        ContractErrorCode::ValidationFailed,
                        "local broker requires peer uid checks and quotas",
                        false,
                    ));
                }
            }
            SocketEndpointRole::HealthMetrics => {
                if endpoint.access_scope != SocketAccessScope::LocalMultiUser {
                    return Err(ContractError::new(
                        ContractErrorCode::ValidationFailed,
                        "health metrics endpoint must use local_multi_user scope",
                        false,
                    ));
                }
                ensure_default_or_deviation(
                    endpoint.path.as_str(),
                    HEALTH_METRICS_SOCKET_PATH_V1,
                    endpoint.deviation_reason.as_ref(),
                    "health metrics socket path",
                )?;
                if !endpoint.nonsecret_metrics_only {
                    return Err(ContractError::new(
                        ContractErrorCode::ValidationFailed,
                        "health metrics endpoint must remain nonsecret-only",
                        false,
                    ));
                }
                if endpoint.allow_cross_user_journal_access {
                    return Err(ContractError::new(
                        ContractErrorCode::ValidationFailed,
                        "health metrics endpoint must not expose unrestricted cross-user journals",
                        false,
                    ));
                }
            }
            SocketEndpointRole::UserRouter => {
                if endpoint.access_scope != SocketAccessScope::PerUser {
                    return Err(ContractError::new(
                        ContractErrorCode::ValidationFailed,
                        "user router endpoint must use per_user scope",
                        false,
                    ));
                }
                if !is_valid_user_router_path(endpoint.path.as_str()) {
                    return Err(ContractError::new(
                        ContractErrorCode::ValidationFailed,
                        "user router socket path must be $XDG_RUNTIME_DIR/sentia/router.sock or /run/user/<uid>/sentia/router.sock",
                        false,
                    ));
                }
                ensure_default_or_deviation(
                    endpoint.socket_mode_octal.as_str(),
                    "0600",
                    endpoint.deviation_reason.as_ref(),
                    "user router socket mode",
                )?;
                if !endpoint.require_peer_uid_check {
                    return Err(ContractError::new(
                        ContractErrorCode::ValidationFailed,
                        "user router endpoint must enforce peer uid checks",
                        false,
                    ));
                }
            }
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
    pub integration_sockets: IntegrationSocketDefaults,
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
        if !is_valid_user_router_path(&self.socket_path) {
            return Err(ContractError::new(
                ContractErrorCode::ValidationFailed,
                "router socket_path must be $XDG_RUNTIME_DIR/sentia/router.sock or /run/user/<uid>/sentia/router.sock",
                false,
            ));
        }
        self.integration_sockets.validate()?;
        if self.socket_path != self.integration_sockets.user_router.path.as_str() {
            return Err(ContractError::new(
                ContractErrorCode::ValidationFailed,
                "router socket_path must match integration_sockets.user_router.path",
                false,
            ));
        }
        Ok(())
    }
}

fn validate_socket_mode(mode: &str, field: &str) -> Result<(), ContractError> {
    let chars: Vec<char> = mode.chars().collect();
    if chars.len() != 4 || chars[0] != '0' || chars[1..].iter().any(|c| !('0'..='7').contains(c)) {
        return Err(ContractError::new(
            ContractErrorCode::ValidationFailed,
            format!("{field} must be a 4-char octal mode like 0700"),
            false,
        ));
    }
    Ok(())
}

fn ensure_default_or_deviation(
    actual: &str,
    expected: &str,
    deviation_reason: Option<&String>,
    label: &str,
) -> Result<(), ContractError> {
    if actual == expected {
        return Ok(());
    }

    if let Some(reason) = deviation_reason {
        if !reason.trim().is_empty() {
            return Ok(());
        }
    }

    Err(ContractError::new(
        ContractErrorCode::ValidationFailed,
        format!("{label} deviates from default and requires a documented deviation_reason"),
        false,
    ))
}

fn is_valid_user_router_path(path: &str) -> bool {
    if path == USER_ROUTER_SOCKET_TEMPLATE_V1 {
        return true;
    }
    if !path.starts_with("/run/user/") || !path.ends_with("/sentia/router.sock") {
        return false;
    }

    let uid_segment = &path["/run/user/".len()..path.len() - "/sentia/router.sock".len()];
    !uid_segment.is_empty() && uid_segment.chars().all(|char_| char_.is_ascii_digit())
}

fn bounded_socket_path(value: &str) -> SocketPath {
    BoundedString::new(value.to_owned()).expect("default socket path should fit bounds")
}

fn bounded_socket_mode(value: &str) -> SocketMode {
    BoundedString::new(value.to_owned()).expect("default socket mode should fit bounds")
}

fn bounded_system_user(value: &str) -> SystemUser {
    BoundedString::new(value.to_owned()).expect("default system user should fit bounds")
}
