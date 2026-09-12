use sentia_protocol::{ContractError, ProviderErrorCode, ProviderHealth};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const BROKER_PROTOCOL_VERSION: &str = sentia_protocol::PROTOCOL_VERSION_V1;
pub const MAX_BROKER_FRAME_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerMessage {
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<BrokerToolCall>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerTool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrokerRequest {
    Chat {
        version: String,
        request_id: String,
        messages: Vec<BrokerMessage>,
        #[serde(default)]
        tools: Vec<BrokerTool>,
        max_tokens: u32,
    },
    Health {
        version: String,
        request_id: String,
    },
    Cancel {
        version: String,
        request_id: String,
        target_request_id: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrokerEvent {
    Queued {
        version: String,
        request_id: String,
        position: u32,
    },
    Progress {
        version: String,
        request_id: String,
        state: String,
        detail: String,
    },
    Delta {
        version: String,
        request_id: String,
        content: String,
    },
    ToolCalls {
        version: String,
        request_id: String,
        calls: Vec<BrokerToolCall>,
    },
    Complete {
        version: String,
        request_id: String,
        finish_reason: String,
    },
    Health {
        version: String,
        request_id: String,
        health: ProviderHealth,
        detail: String,
    },
    Cancelled {
        version: String,
        request_id: String,
        target_request_id: String,
    },
    Error {
        version: String,
        request_id: String,
        error: ContractError,
        provider_error: ProviderErrorCode,
    },
}
