use serde::{Deserialize, Serialize};

pub const SYSTEM1_BUS_NAME: &str = "org.sentia.System1";
pub const SYSTEM1_OBJECT_PATH: &str = "/org/sentia/System1";
pub const SYSTEM1_INTERFACE: &str = "org.sentia.System1";
pub const SYSTEM1_METHOD_PREPARE: &str = "Prepare";
pub const SYSTEM1_METHOD_APPLY: &str = "Apply";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceAction {
    Start,
    Stop,
    Restart,
    Enable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum PrivilegedOperation {
    Service { action: ServiceAction, unit: String },
    ProcessKill { pid: u32 },
    AptInstall { packages: Vec<String> },
    AptRemove { packages: Vec<String> },
    AptUpdate,
    AptUpgrade,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareRequest {
    pub version: u32,
    pub operation: PrivilegedOperation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareResponse {
    pub plan: PreparedPlan,
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedPlan {
    pub version: u32,
    pub id: String,
    pub caller: PreparedCaller,
    pub action_id: String,
    pub operation: PrivilegedOperation,
    pub argument_digest: String,
    pub state: serde_json::Value,
    pub issued_at: u64,
    pub expires_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedCaller {
    pub unique_name: String,
    pub uid: u32,
    pub pid: u32,
    pub process_start: u64,
    pub session: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyCall {
    pub id: String,
    pub digest: String,
}

#[derive(Debug, Clone)]
pub struct PreparedPlanTicket {
    dbus_unique_name: String,
    consumed: bool,
    exact_prepare_json: String,
    response: PrepareResponse,
}

impl PreparedPlanTicket {
    pub fn from_prepare_json(
        dbus_unique_name: impl Into<String>,
        exact_prepare_json: String,
    ) -> Result<Self, serde_json::Error> {
        let response: PrepareResponse = serde_json::from_str(&exact_prepare_json)?;
        Ok(Self {
            dbus_unique_name: dbus_unique_name.into(),
            consumed: false,
            exact_prepare_json,
            response,
        })
    }

    pub fn new(dbus_unique_name: impl Into<String>, response: PrepareResponse) -> Self {
        let exact_prepare_json =
            serde_json::to_string_pretty(&response).expect("prepare response should serialize");
        Self {
            dbus_unique_name: dbus_unique_name.into(),
            consumed: false,
            exact_prepare_json,
            response,
        }
    }

    pub fn exact_preview_json(&self) -> &str {
        &self.exact_prepare_json
    }

    pub fn response(&self) -> &PrepareResponse {
        &self.response
    }

    pub fn apply_call(&self) -> ApplyCall {
        ApplyCall {
            id: self.response.plan.id.clone(),
            digest: self.response.digest.clone(),
        }
    }

    pub fn consume_for_apply(
        &mut self,
        dbus_unique_name: &str,
        id: &str,
        digest: &str,
    ) -> Result<(), ApplyGuardError> {
        if dbus_unique_name.trim().is_empty() {
            return Err(ApplyGuardError::MissingConnection);
        }

        if self.consumed {
            return Err(ApplyGuardError::AlreadyConsumed);
        }

        if self.dbus_unique_name != dbus_unique_name {
            return Err(ApplyGuardError::ConnectionChanged {
                expected: self.dbus_unique_name.clone(),
                got: dbus_unique_name.to_string(),
            });
        }

        if self.response.plan.id != id {
            return Err(ApplyGuardError::PlanIdMismatch {
                expected: self.response.plan.id.clone(),
                got: id.to_string(),
            });
        }

        if self.response.digest != digest {
            return Err(ApplyGuardError::DigestMismatch);
        }

        self.consumed = true;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyGuardError {
    MissingConnection,
    AlreadyConsumed,
    ConnectionChanged { expected: String, got: String },
    PlanIdMismatch { expected: String, got: String },
    DigestMismatch,
}

impl std::fmt::Display for ApplyGuardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApplyGuardError::MissingConnection => {
                f.write_str("missing D-Bus unique connection name")
            }
            ApplyGuardError::AlreadyConsumed => {
                f.write_str("prepared plan already consumed; prepare again before apply")
            }
            ApplyGuardError::ConnectionChanged { expected, got } => {
                write!(
                    f,
                    "prepare/apply must use same D-Bus connection (expected {expected}, got {got})"
                )
            }
            ApplyGuardError::PlanIdMismatch { expected, got } => {
                write!(f, "plan ID mismatch (expected {expected}, got {got})")
            }
            ApplyGuardError::DigestMismatch => {
                f.write_str("plan digest mismatch; previewed plan no longer matches")
            }
        }
    }
}

impl std::error::Error for ApplyGuardError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_response() -> PrepareResponse {
        PrepareResponse {
            plan: PreparedPlan {
                version: 1,
                id: "abc123".to_string(),
                caller: PreparedCaller {
                    unique_name: ":1.42".to_string(),
                    uid: 1000,
                    pid: 4242,
                    process_start: 98765,
                    session: "/org/freedesktop/login1/session/_31".to_string(),
                },
                action_id: "org.sentia.system.service-restart".to_string(),
                operation: PrivilegedOperation::Service {
                    action: ServiceAction::Restart,
                    unit: "ssh.service".to_string(),
                },
                argument_digest: "arg-digest".to_string(),
                state: serde_json::json!({"active": "active"}),
                issued_at: 1_700_000_000,
                expires_at: 1_700_000_120,
            },
            digest: "outer-digest".to_string(),
        }
    }

    #[test]
    fn prepare_request_serializes_service_schema() {
        let request = PrepareRequest {
            version: 1,
            operation: PrivilegedOperation::Service {
                action: ServiceAction::Restart,
                unit: "ssh.service".to_string(),
            },
        };

        let json = serde_json::to_value(request).expect("serialize request");
        assert_eq!(
            json,
            serde_json::json!({
                "version": 1,
                "operation": {
                    "operation": "service",
                    "action": "restart",
                    "unit": "ssh.service"
                }
            })
        );
    }

    #[test]
    fn prepare_request_rejects_approved_field() {
        let raw = r#"{"version":1,"approved":true,"operation":{"operation":"apt_update"}}"#;
        let parsed = serde_json::from_str::<PrepareRequest>(raw);
        assert!(parsed.is_err());
    }

    #[test]
    fn prepare_response_rejects_unknown_fields() {
        let raw = r#"{
            "plan": {
              "version": 1,
              "id": "id-1",
              "caller": {
                "unique_name": ":1.9",
                "uid": 1000,
                "pid": 99,
                "process_start": 123,
                "session": "/org/freedesktop/login1/session/_2"
              },
              "action_id": "org.sentia.system.service-restart",
              "operation": {"operation":"service","action":"restart","unit":"ssh.service"},
              "argument_digest": "a",
              "state": {},
              "issued_at": 1,
              "expires_at": 2,
              "approved": true
            },
            "digest": "d"
        }"#;

        let parsed = serde_json::from_str::<PrepareResponse>(raw);
        assert!(parsed.is_err());
    }

    #[test]
    fn apply_guard_requires_same_connection_and_single_use() {
        let response = sample_response();
        let mut ticket = PreparedPlanTicket::new(":1.42", response);

        ticket
            .consume_for_apply(":1.42", "abc123", "outer-digest")
            .expect("first apply should be allowed");

        let second = ticket.consume_for_apply(":1.42", "abc123", "outer-digest");
        assert!(matches!(second, Err(ApplyGuardError::AlreadyConsumed)));
    }

    #[test]
    fn apply_guard_rejects_connection_change() {
        let response = sample_response();
        let mut ticket = PreparedPlanTicket::new(":1.1", response);
        let result = ticket.consume_for_apply(":1.2", "abc123", "outer-digest");

        assert!(matches!(
            result,
            Err(ApplyGuardError::ConnectionChanged { .. })
        ));
    }

    #[test]
    fn apply_guard_rejects_mismatched_digest() {
        let response = sample_response();
        let mut ticket = PreparedPlanTicket::new(":1.77", response);
        let result = ticket.consume_for_apply(":1.77", "abc123", "bad-digest");
        assert!(matches!(result, Err(ApplyGuardError::DigestMismatch)));
    }

    #[test]
    fn ticket_from_prepare_json_preserves_exact_preview() {
        let raw = r#"{"plan":{"version":1,"id":"id-7","caller":{"unique_name":":1.5","uid":1000,"pid":55,"process_start":1234,"session":"/org/freedesktop/login1/session/_3"},"action_id":"org.sentia.system.apt-update","operation":{"operation":"apt_update"},"argument_digest":"abc","state":{"network":"online"},"issued_at":1700000000,"expires_at":1700000120},"digest":"final-digest"}"#;

        let ticket = PreparedPlanTicket::from_prepare_json(":1.5", raw.to_string())
            .expect("prepare response should parse");

        assert_eq!(ticket.exact_preview_json(), raw);
        let apply = ticket.apply_call();
        assert_eq!(apply.id, "id-7");
        assert_eq!(apply.digest, "final-digest");
    }
}
