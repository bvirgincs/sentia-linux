use sentia_protocol::{
    ApprovalBinding, ApprovalGrant, ApprovalPlan, AuthorizationApplyRequest, AuthorizationApplyResult,
    AuthorizationOperation, AuthorizationPrepareRequest, AuthorizationPrepareResult, BoundedString,
    CanonicalTransactionPlan, ContractError, ContractErrorCode, PackageDelta, RequiredEvidence,
    TransactionKind,
};

fn bounded<const MAX: usize>(value: &str) -> BoundedString<MAX> {
    BoundedString::new(value.to_owned()).expect("value should fit bounded string")
}

fn digest(char_: char) -> String {
    std::iter::repeat(char_).take(64).collect()
}

fn sample_approval_plan() -> ApprovalPlan {
    ApprovalPlan {
        version: bounded("sentia.v1"),
        approval_id: bounded("approval-a1"),
        transaction_plan: CanonicalTransactionPlan {
            version: bounded("sentia.v1"),
            plan_id: bounded("plan-a1"),
            created_at_ms: 10,
            requester_uid: 1000,
            kind: TransactionKind::AptInstall,
            requested_targets: vec![bounded("curl")],
            package_changes: vec![PackageDelta {
                package: bounded("curl"),
                from_version: Some(bounded("8.7.1-1")),
                to_version: Some(bounded("8.7.1-2")),
                download_bytes: 120_000,
                installed_size_delta_bytes: 64_000,
                service_effect: None,
            }],
            service_changes: vec![],
            unknown_effects: vec![],
            state_fingerprint_sha256: bounded(&digest('1')),
            approval_required: true,
        },
        binding: ApprovalBinding {
            session_id: bounded("session-1"),
            caller_uid: 1000,
            operation_digest_sha256: bounded(&digest('2')),
            relevant_state_digest_sha256: bounded(&digest('3')),
            issued_at_ms: 20,
            expires_at_ms: 40,
            single_use: true,
            polkit_action: bounded("org.sentia.System1.Apply"),
        },
        required_evidence: vec![RequiredEvidence {
            case_id: bounded("TOOLS-REGISTRY-SCHEMA-COVERAGE"),
            rationale: bounded("mutation requires evidence linkage"),
        }],
    }
}

#[test]
fn prepare_request_requires_requested_targets() {
    let request = AuthorizationPrepareRequest {
        version: bounded("sentia.v1"),
        request_id: bounded("auth-req-1"),
        session_id: bounded("session-1"),
        caller_uid: 1000,
        kind: TransactionKind::AptInstall,
        requested_targets: vec![],
        state_fingerprint_sha256: bounded(&digest('4')),
        require_non_cached_auth: true,
    };

    let error = request
        .validate()
        .expect_err("prepare request should reject empty targets");
    assert_eq!(error.code, ContractErrorCode::ValidationFailed);
}

#[test]
fn prepare_result_requires_prepare_operation() {
    let result = AuthorizationPrepareResult {
        version: bounded("sentia.v1"),
        request_id: bounded("auth-req-2"),
        operation: AuthorizationOperation::Apply,
        approval_plan: sample_approval_plan(),
    };

    let error = result
        .validate()
        .expect_err("prepare result should require operation=prepare");
    assert_eq!(error.code, ContractErrorCode::ValidationFailed);
}

#[test]
fn apply_request_rejects_expired_grant() {
    let request = AuthorizationApplyRequest {
        version: bounded("sentia.v1"),
        request_id: bounded("auth-req-3"),
        session_id: bounded("session-1"),
        caller_uid: 1000,
        approval_grant: ApprovalGrant {
            approval_id: bounded("approval-a1"),
            grant_id: bounded("grant-1"),
            broker_token: bounded("token-abc"),
            granted_at_ms: 50,
            expires_at_ms: 50,
        },
        expected_operation_digest_sha256: bounded(&digest('5')),
        expected_state_digest_sha256: bounded(&digest('6')),
    };

    let error = request
        .validate()
        .expect_err("expired grant should fail apply request validation");
    assert_eq!(error.code, ContractErrorCode::ValidationFailed);
}

#[test]
fn apply_result_requires_error_when_not_applied() {
    let result = AuthorizationApplyResult {
        version: bounded("sentia.v1"),
        request_id: bounded("auth-req-4"),
        operation: AuthorizationOperation::Apply,
        approval_id: bounded("approval-a1"),
        applied: false,
        applied_at_ms: 70,
        evidence: vec![],
        error: None,
    };

    let error = result
        .validate()
        .expect_err("non-applied result requires structured error");
    assert_eq!(error.code, ContractErrorCode::ValidationFailed);
}

#[test]
fn apply_result_serializes_without_client_approval_bool() {
    let result = AuthorizationApplyResult {
        version: bounded("sentia.v1"),
        request_id: bounded("auth-req-5"),
        operation: AuthorizationOperation::Apply,
        approval_id: bounded("approval-a1"),
        applied: true,
        applied_at_ms: 80,
        evidence: vec![],
        error: None,
    };

    result.validate().expect("applied result should validate");
    let encoded = serde_json::to_value(result).expect("serialize apply result");
    assert!(
        encoded.get("approved").is_none(),
        "authorization contracts must not expose client approval booleans"
    );
}

#[test]
fn apply_result_with_failure_error_validates() {
    let result = AuthorizationApplyResult {
        version: bounded("sentia.v1"),
        request_id: bounded("auth-req-6"),
        operation: AuthorizationOperation::Apply,
        approval_id: bounded("approval-a1"),
        applied: false,
        applied_at_ms: 81,
        evidence: vec![],
        error: Some(ContractError::new(
            ContractErrorCode::PermissionDenied,
            "polkit denied operation",
            false,
        )),
    };

    result
        .validate()
        .expect("failure result with structured error should validate");
}
