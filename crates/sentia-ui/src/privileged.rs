use serde::{Deserialize, Serialize};

pub const SYSTEM1_BUS_NAME: &str = "org.sentia.System1";
pub const SYSTEM1_OBJECT_PATH: &str = "/org/sentia/System1";
pub const SYSTEM1_INTERFACE: &str = "org.sentia.System1";
pub const SYSTEM1_METHOD_PREPARE: &str = "Prepare";
pub const SYSTEM1_METHOD_APPLY: &str = "Apply";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivilegedAction {
    AuthAdmin,
}

impl PrivilegedAction {
    pub fn as_label(self) -> &'static str {
        match self {
            PrivilegedAction::AuthAdmin => "auth_admin",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokerPrepareRequest {
    pub action: PrivilegedAction,
    pub request_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokerPreparedPlan {
    pub plan_id: String,
    pub digest: String,
    pub preview: String,
    pub typed_confirmation: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PreparedPlanTicket {
    dbus_unique_name: String,
    consumed: bool,
    plan: BrokerPreparedPlan,
}

impl PreparedPlanTicket {
    pub fn new(dbus_unique_name: impl Into<String>, plan: BrokerPreparedPlan) -> Self {
        Self {
            dbus_unique_name: dbus_unique_name.into(),
            consumed: false,
            plan,
        }
    }

    pub fn plan(&self) -> &BrokerPreparedPlan {
        &self.plan
    }

    pub fn consume_for_apply(
        &mut self,
        dbus_unique_name: &str,
        plan_id: &str,
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

        if self.plan.plan_id != plan_id {
            return Err(ApplyGuardError::PlanIdMismatch {
                expected: self.plan.plan_id.clone(),
                got: plan_id.to_string(),
            });
        }

        if self.plan.digest != digest {
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

    #[test]
    fn apply_guard_requires_same_connection_and_single_use() {
        let plan = BrokerPreparedPlan {
            plan_id: "plan-1".to_string(),
            digest: "digest-abc".to_string(),
            preview: "Install package X".to_string(),
            typed_confirmation: Some("CONFIRM".to_string()),
        };

        let mut ticket = PreparedPlanTicket::new(":1.42", plan);

        ticket
            .consume_for_apply(":1.42", "plan-1", "digest-abc")
            .expect("first apply should be allowed");

        let second = ticket.consume_for_apply(":1.42", "plan-1", "digest-abc");
        assert!(matches!(second, Err(ApplyGuardError::AlreadyConsumed)));
    }

    #[test]
    fn apply_guard_rejects_connection_change() {
        let plan = BrokerPreparedPlan {
            plan_id: "plan-9".to_string(),
            digest: "digest-xyz".to_string(),
            preview: "Restart service Y".to_string(),
            typed_confirmation: None,
        };

        let mut ticket = PreparedPlanTicket::new(":1.1", plan);
        let result = ticket.consume_for_apply(":1.2", "plan-9", "digest-xyz");

        assert!(matches!(
            result,
            Err(ApplyGuardError::ConnectionChanged { .. })
        ));
    }

    #[test]
    fn apply_guard_rejects_mismatched_digest() {
        let plan = BrokerPreparedPlan {
            plan_id: "plan-2".to_string(),
            digest: "digest-good".to_string(),
            preview: "Apply policy".to_string(),
            typed_confirmation: None,
        };

        let mut ticket = PreparedPlanTicket::new(":1.77", plan);
        let result = ticket.consume_for_apply(":1.77", "plan-2", "digest-bad");
        assert!(matches!(result, Err(ApplyGuardError::DigestMismatch)));
    }
}
