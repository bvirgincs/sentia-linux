#![forbid(unsafe_code)]

pub mod bounded;
pub mod errors;
pub mod evidence;
pub mod metrics;
pub mod provider;
pub mod router;
pub mod socket;
pub mod tools;
pub mod transactions;

pub use bounded::{BoundedString, BoundedStringError, BoundedVec, BoundedVecError};
pub use errors::{ContractError, ContractErrorCode};
pub use evidence::{EvidenceDomain, EvidencePointer, EvidenceRecord, EvidenceStatus};
pub use metrics::{MetricSample, MetricUnit, RouterMetrics};
pub use provider::{
    ProviderCapabilities, ProviderCapability, ProviderError, ProviderErrorCode, ProviderHealth,
    ProviderState, ProviderStatus,
};
pub use router::{
    DataProvenance, DataSensitivity, IntegrationSocketDefaults, RequestPayload, RequestStatus,
    ResultPayload, RouterCapability, RouterConfig, RouterRequest, RouterResult, RoutingPolicy,
    SocketAccessScope, SocketEndpointPolicy, SocketEndpointRole, SocketMode, SocketPath,
    SystemUser, HEALTH_METRICS_SOCKET_PATH_V1, INFERENCE_SOCKET_PATH_V1,
    LOCAL_BROKER_SOCKET_PATH_V1, PROTOCOL_VERSION_V1, USER_ROUTER_SOCKET_TEMPLATE_V1,
};
pub use socket::{
    CancelAck, CancelReason, CancelRequest, EventPayload, JsonlFrame, StreamEvent, ToolPhase,
    UnixSocketSecurity, SOCKET_TRANSPORT_V1,
};
pub use tools::{
    CancellationPolicy, PrivilegeClass, PrivacyClass, ToolArea, ToolDefinition, ToolEvidence,
    ToolEvidenceKind, ToolName, ToolProvenance, ToolProvenanceSource, ToolRegistry, ToolRequest,
    ToolResult, ToolResultStatus, MAX_TOOL_INPUT_BYTES_V1, MAX_TOOL_OUTPUT_BYTES_V1,
};
pub use transactions::{
    ApprovalBinding, ApprovalGrant, ApprovalPlan, CanonicalTransactionPlan, PackageDelta,
    RequiredEvidence, ServiceDelta, TransactionKind,
};
