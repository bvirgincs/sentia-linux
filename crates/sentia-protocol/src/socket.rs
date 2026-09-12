use crate::bounded::BoundedString;
use crate::errors::ContractError;
use crate::provider::ProviderStatus;
use crate::router::{RequestId, RequestStatus, RouterRequest, RouterResult, PROTOCOL_VERSION_V1};
use crate::tools::{ToolName, ToolRequest, ToolResult};
use serde::{Deserialize, Serialize};

pub const SOCKET_TRANSPORT_V1: &str = "jsonl-unix-v1";

pub type FrameId = BoundedString<64>;
pub type StreamId = BoundedString<64>;
pub type EventId = BoundedString<64>;
pub type CancelId = BoundedString<64>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnixSocketSecurity {
    pub transport: String,
    pub runtime_dir_only: bool,
    pub require_peer_credentials: bool,
    pub directory_mode_octal: String,
    pub socket_mode_octal: String,
}

impl Default for UnixSocketSecurity {
    fn default() -> Self {
        Self {
            transport: SOCKET_TRANSPORT_V1.to_owned(),
            runtime_dir_only: true,
            require_peer_credentials: true,
            directory_mode_octal: "0700".to_owned(),
            socket_mode_octal: "0600".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPhase {
    Queued,
    Validating,
    Running,
    Verifying,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum EventPayload {
    StateTransition {
        from: RequestStatus,
        to: RequestStatus,
    },
    OutputDelta {
        delta: BoundedString<8192>,
        cumulative_bytes: u32,
    },
    ProviderStatus {
        provider_status: ProviderStatus,
    },
    ToolProgress {
        tool: ToolName,
        phase: ToolPhase,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    Terminal {
        final_status: RequestStatus,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamEvent {
    pub event_id: EventId,
    pub request_id: RequestId,
    pub sequence: u64,
    pub status: RequestStatus,
    pub emitted_at_ms: u64,
    pub payload: EventPayload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelReason {
    UserRequested,
    Timeout,
    ClientDisconnect,
    ServiceShutdown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancelRequest {
    pub cancel_id: CancelId,
    pub request_id: RequestId,
    pub reason: CancelReason,
    pub requested_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancelAck {
    pub cancel_id: CancelId,
    pub request_id: RequestId,
    pub accepted: bool,
    pub final_status: RequestStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "frame_type", rename_all = "snake_case")]
pub enum JsonlFrame {
    Request {
        version: String,
        frame_id: FrameId,
        stream_id: StreamId,
        request: RouterRequest,
    },
    Event {
        version: String,
        frame_id: FrameId,
        stream_id: StreamId,
        event: StreamEvent,
    },
    Result {
        version: String,
        frame_id: FrameId,
        stream_id: StreamId,
        result: RouterResult,
    },
    Error {
        version: String,
        frame_id: FrameId,
        stream_id: StreamId,
        error: ContractError,
    },
    Cancel {
        version: String,
        frame_id: FrameId,
        stream_id: StreamId,
        cancel: CancelRequest,
    },
    CancelAck {
        version: String,
        frame_id: FrameId,
        stream_id: StreamId,
        cancel_ack: CancelAck,
    },
    ToolRequest {
        version: String,
        frame_id: FrameId,
        stream_id: StreamId,
        tool_request: ToolRequest,
    },
    ToolResult {
        version: String,
        frame_id: FrameId,
        stream_id: StreamId,
        tool_result: ToolResult,
    },
}

impl JsonlFrame {
    pub fn validate_version(&self) -> Result<(), ContractError> {
        let version = match self {
            JsonlFrame::Request { version, .. }
            | JsonlFrame::Event { version, .. }
            | JsonlFrame::Result { version, .. }
            | JsonlFrame::Error { version, .. }
            | JsonlFrame::Cancel { version, .. }
            | JsonlFrame::CancelAck { version, .. }
            | JsonlFrame::ToolRequest { version, .. }
            | JsonlFrame::ToolResult { version, .. } => version,
        };

        if version == PROTOCOL_VERSION_V1 {
            Ok(())
        } else {
            Err(ContractError::new(
                crate::errors::ContractErrorCode::UnsupportedVersion,
                "frame version is not sentia.v1",
                false,
            ))
        }
    }
}
