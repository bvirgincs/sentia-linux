use crate::bounded::{BoundedString, BoundedVec};
use serde::{Deserialize, Serialize};

pub type EvidenceId = BoundedString<80>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum EvidenceDomain {
    BUILD,
    LIVE,
    INSTALL,
    DISK,
    OFFLINE,
    AI,
    TOOLS,
    SHELL,
    HEALTH,
    ROUTER,
    PRIVACY,
    TRUST,
    SECUREBOOT,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStatus {
    Missing,
    Collected,
    Verified,
    Failed,
    Redacted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidencePointerKind {
    IsoChecksum,
    BuildLog,
    SerialLog,
    JournalLog,
    Screenshot,
    ArtifactManifest,
    TestReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidencePointer {
    pub kind: EvidencePointerKind,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRecord {
    pub evidence_id: EvidenceId,
    pub domain: EvidenceDomain,
    pub required: bool,
    pub status: EvidenceStatus,
    pub assertion: String,
    pub pointers: BoundedVec<EvidencePointer, 32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
}
