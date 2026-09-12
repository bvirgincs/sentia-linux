use sentia_protocol::{
    bounded::BoundedString,
    transactions::{
        ApprovalBinding, ApprovalPlan, CanonicalTransactionPlan, PackageDelta, RequiredEvidence,
        TransactionKind,
    },
};

fn bounded<const MAX: usize>(value: &str) -> BoundedString<MAX> {
    BoundedString::new(value.to_owned()).expect("value should be in bounds")
}

fn digest(char_: char) -> String {
    std::iter::repeat(char_).take(64).collect()
}

fn sample_plan() -> CanonicalTransactionPlan {
    CanonicalTransactionPlan {
        version: bounded("sentia.v1"),
        plan_id: bounded("plan-1"),
        created_at_ms: 1234,
        requester_uid: 1000,
        kind: TransactionKind::AptInstall,
        requested_targets: vec![bounded("vim"), bounded("curl")],
        package_changes: vec![
            PackageDelta {
                package: bounded("vim"),
                from_version: None,
                to_version: Some(bounded("2:9.1-1")),
                download_bytes: 600_000,
                installed_size_delta_bytes: 2_000_000,
                service_effect: None,
            },
            PackageDelta {
                package: bounded("curl"),
                from_version: Some(bounded("8.7.1-1")),
                to_version: Some(bounded("8.7.1-2")),
                download_bytes: 120_000,
                installed_size_delta_bytes: 64_000,
                service_effect: None,
            },
        ],
        service_changes: vec![],
        unknown_effects: vec![bounded("manual review required")],
        state_fingerprint_sha256: bounded(&digest('a')),
        approval_required: true,
    }
}

#[test]
fn canonical_transaction_json_is_stable_across_input_order() {
    let first = sample_plan();
    let mut second = sample_plan();
    second.requested_targets.reverse();
    second.package_changes.reverse();

    let first_json = first.canonical_json().expect("canonical serialization");
    let second_json = second.canonical_json().expect("canonical serialization");

    assert_eq!(first_json, second_json);
    assert!(
        first_json.contains("\"requested_targets\":[\"curl\",\"vim\"]"),
        "requested targets should be canonically sorted"
    );
    assert!(
        first_json.contains("\"package\":\"curl\""),
        "curl delta should be serialized first after canonicalization"
    );
}

#[test]
fn approval_plan_has_binding_digest_and_no_client_approved_flag() {
    let plan = sample_plan();
    let approval = ApprovalPlan {
        version: bounded("sentia.v1"),
        approval_id: bounded("approval-1"),
        transaction_plan: plan,
        binding: ApprovalBinding {
            session_id: bounded("session-1"),
            caller_uid: 1000,
            operation_digest_sha256: bounded(&digest('b')),
            relevant_state_digest_sha256: bounded(&digest('c')),
            issued_at_ms: 100,
            expires_at_ms: 200,
            single_use: true,
            polkit_action: bounded("org.sentia.apt.install"),
        },
        required_evidence: vec![RequiredEvidence {
            case_id: bounded("TOOLS-REGISTRY-SCHEMA-COVERAGE"),
            rationale: bounded("requires canonical review before mutation"),
        }],
    };

    approval.validate().expect("approval plan validation");
    let serialized = serde_json::to_value(&approval).expect("approval serialization");
    assert!(
        serialized.get("approved").is_none(),
        "client-approved flags are forbidden in approval contract"
    );
    assert!(
        serialized["binding"].get("approved").is_none(),
        "approval authority must be bound to broker output, not client booleans"
    );
}
