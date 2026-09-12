use crate::bounded::BoundedString;
use crate::errors::{ContractError, ContractErrorCode};
use crate::router::PROTOCOL_VERSION_V1;
use serde::{Deserialize, Serialize};

pub type PlanId = BoundedString<64>;
pub type ApprovalId = BoundedString<64>;
pub type Sha256Digest = BoundedString<64>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionKind {
    AptInstall,
    AptRemove,
    AptUpdate,
    AptUpgrade,
    ServiceStart,
    ServiceStop,
    ServiceRestart,
    ServiceEnable,
    ProcessKill,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageDelta {
    pub package: BoundedString<128>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_version: Option<BoundedString<64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_version: Option<BoundedString<64>>,
    pub download_bytes: u64,
    pub installed_size_delta_bytes: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_effect: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceDelta {
    pub service: BoundedString<128>,
    pub action: BoundedString<64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_state: Option<BoundedString<32>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalTransactionPlan {
    pub version: BoundedString<24>,
    pub plan_id: PlanId,
    pub created_at_ms: u64,
    pub requester_uid: u32,
    pub kind: TransactionKind,
    pub requested_targets: Vec<BoundedString<128>>,
    pub package_changes: Vec<PackageDelta>,
    pub service_changes: Vec<ServiceDelta>,
    pub unknown_effects: Vec<BoundedString<256>>,
    pub state_fingerprint_sha256: Sha256Digest,
    pub approval_required: bool,
}

impl CanonicalTransactionPlan {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.version.as_str() != PROTOCOL_VERSION_V1 {
            return Err(ContractError::new(
                ContractErrorCode::UnsupportedVersion,
                "transaction plan version is not sentia.v1",
                false,
            ));
        }
        if self.requested_targets.is_empty()
            && self.package_changes.is_empty()
            && self.service_changes.is_empty()
        {
            return Err(ContractError::new(
                ContractErrorCode::ValidationFailed,
                "transaction plan has no targets or changes",
                false,
            ));
        }
        Ok(())
    }

    pub fn canonicalized(&self) -> Self {
        let mut canonical = self.clone();
        canonical.requested_targets.sort();
        canonical.unknown_effects.sort();
        canonical
            .package_changes
            .sort_by(|a, b| a.package.cmp(&b.package).then(a.to_version.cmp(&b.to_version)));
        canonical
            .service_changes
            .sort_by(|a, b| a.service.cmp(&b.service).then(a.action.cmp(&b.action)));
        canonical
    }

    pub fn canonical_json(&self) -> Result<String, ContractError> {
        self.validate()?;
        let canonical = self.canonicalized();
        serde_json::to_string(&canonical).map_err(|e| {
            ContractError::new(
                ContractErrorCode::Internal,
                format!("failed to serialize canonical transaction plan: {e}"),
                false,
            )
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequiredEvidence {
    pub case_id: BoundedString<64>,
    pub rationale: BoundedString<256>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalBinding {
    pub session_id: BoundedString<64>,
    pub caller_uid: u32,
    pub operation_digest_sha256: Sha256Digest,
    pub relevant_state_digest_sha256: Sha256Digest,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    pub single_use: bool,
    pub polkit_action: BoundedString<128>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalPlan {
    pub version: BoundedString<24>,
    pub approval_id: ApprovalId,
    pub transaction_plan: CanonicalTransactionPlan,
    pub binding: ApprovalBinding,
    pub required_evidence: Vec<RequiredEvidence>,
}

impl ApprovalPlan {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.version.as_str() != PROTOCOL_VERSION_V1 {
            return Err(ContractError::new(
                ContractErrorCode::UnsupportedVersion,
                "approval plan version is not sentia.v1",
                false,
            ));
        }
        self.transaction_plan.validate()?;
        if self.binding.expires_at_ms <= self.binding.issued_at_ms {
            return Err(ContractError::new(
                ContractErrorCode::ValidationFailed,
                "approval binding expiry must be after issued timestamp",
                false,
            ));
        }
        if self.required_evidence.is_empty() {
            return Err(ContractError::new(
                ContractErrorCode::ValidationFailed,
                "approval plan requires at least one evidence requirement",
                false,
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalGrant {
    pub approval_id: ApprovalId,
    pub grant_id: BoundedString<64>,
    pub broker_token: BoundedString<128>,
    pub granted_at_ms: u64,
    pub expires_at_ms: u64,
}
