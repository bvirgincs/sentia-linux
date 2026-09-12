use sentia_protocol::{
    ContractError, DataProvenance, ProviderErrorCode, ProviderStatus, RequestStatus, RoutingPolicy,
};
use sentia_protocol::router::ConsentToken;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

pub const MAX_FRAME_BYTES: usize = 1024 * 1024;
pub const CHAT_MIME: &str = "application/vnd.sentia.chat.v1+json";
pub const CONTROL_MIME: &str = "application/vnd.sentia.control.v1+json";
pub const STATUS_MIME: &str = "application/vnd.sentia.status.v1+json";
pub const SETTINGS_MIME: &str = "application/vnd.sentia.settings.v1+json";
pub const CONSENT_MIME: &str = "application/vnd.sentia.consent-preview.v1+json";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Ord, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyCategory {
    UserPrompt,
    FileContent,
    CommandHistory,
    Hostname,
    IpAddress,
    ProcessName,
    Journal,
    Diagnostic,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextItem {
    pub category: PrivacyCategory,
    pub source: String,
    pub content: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChatContent {
    pub question: String,
    #[serde(default)]
    pub context: Vec<ContextItem>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    pub provenance: DataProvenance,
}

#[derive(Clone, Debug)]
pub struct ChatInput {
    pub request_id: String,
    pub session_id: String,
    pub question: String,
    pub policy: RoutingPolicy,
    pub provider: Option<String>,
    pub context: Vec<ContextItem>,
    pub consent_token: Option<ConsentToken>,
    pub max_tokens: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsPatch {
    #[serde(default)]
    pub default_policy: Option<RoutingPolicy>,
    #[serde(default)]
    pub preferred_remote_provider: Option<Option<String>>,
    #[serde(default)]
    pub remote_categories: Option<BTreeSet<PrivacyCategory>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UserSettings {
    pub version: String,
    pub default_policy: RoutingPolicy,
    pub preferred_remote_provider: Option<String>,
    pub remote_categories: BTreeSet<PrivacyCategory>,
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            version: sentia_protocol::PROTOCOL_VERSION_V1.to_owned(),
            default_policy: RoutingPolicy::LocalOnly,
            preferred_remote_provider: None,
            remote_categories: BTreeSet::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum ControlRequest {
    Status,
    SettingsGet,
    SettingsUpdate { patch: SettingsPatch },
    ConsentPreview {
        provider: String,
        question: String,
        #[serde(default)]
        context: Vec<ContextItem>,
        #[serde(default)]
        max_tokens: Option<u32>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ControlResponse {
    Status {
        display: String,
        local: ProviderStatus,
        remotes: Vec<ProviderStatus>,
    },
    Settings {
        settings: UserSettings,
    },
    ConsentPreview {
        provider: String,
        token: ConsentToken,
        payload: Value,
        redactions: Vec<String>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureKind {
    Provider(ProviderErrorCode),
    PrivacyDenied,
    Cancelled,
}

impl FailureKind {
    pub fn provider_code(self) -> ProviderErrorCode {
        match self {
            Self::Provider(code) => code,
            Self::PrivacyDenied => ProviderErrorCode::Internal,
            Self::Cancelled => ProviderErrorCode::Internal,
        }
    }
}

#[derive(Clone, Debug)]
pub enum InternalEvent {
    Accepted {
        request_id: String,
        policy: RoutingPolicy,
    },
    StateTransition {
        request_id: String,
        from: RequestStatus,
        to: RequestStatus,
    },
    Progress {
        request_id: String,
        state: String,
        detail: String,
    },
    AnswerStarted {
        request_id: String,
        answer_id: String,
        provider_status: ProviderStatus,
        fallback_reason: Option<FailureKind>,
    },
    Delta {
        request_id: String,
        answer_id: String,
        content: String,
    },
    ToolResult {
        request_id: String,
        call_id: String,
        tool: String,
        provenance: String,
    },
    ActionProposal {
        request_id: String,
        call_id: String,
        tool: String,
        arguments: Value,
        notice: String,
    },
    AnswerFinished {
        request_id: String,
        answer_id: String,
        provider: String,
        incomplete: bool,
        finish_reason: String,
    },
    Control {
        request_id: String,
        response: ControlResponse,
    },
    Cancelled {
        request_id: String,
        target_request_id: String,
    },
    Error {
        request_id: String,
        error: ContractError,
        failure: Option<FailureKind>,
    },
    Done {
        request_id: String,
    },
}
