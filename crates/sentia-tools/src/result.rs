use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PrivilegeClass {
    UserReadOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyClass {
    PublicMetadata,
    SystemMetadata,
    PotentiallySensitive,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Provenance {
    pub source: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolResult {
    pub tool_name: String,
    pub timestamp_unix_ms: u64,
    pub privilege_class: PrivilegeClass,
    pub privacy_class: PrivacyClass,
    pub timeout_ms: u64,
    pub cancellation_policy: String,
    pub provenance: Vec<Provenance>,
    pub partial: bool,
    pub data: Value,
}

impl ToolResult {
    pub fn new(
        tool_name: impl Into<String>,
        privacy_class: PrivacyClass,
        timeout: Duration,
        provenance: Vec<Provenance>,
        partial: bool,
        data: Value,
    ) -> Self {
        let timeout_ms = timeout
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX);

        Self {
            tool_name: tool_name.into(),
            timestamp_unix_ms: unix_time_millis(),
            privilege_class: PrivilegeClass::UserReadOnly,
            privacy_class,
            timeout_ms,
            cancellation_policy:
                "caller-cancellable-before-spawn; child process killed on timeout".to_string(),
            provenance,
            partial,
            data,
        }
    }
}

fn unix_time_millis() -> u64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    millis.try_into().unwrap_or(u64::MAX)
}
