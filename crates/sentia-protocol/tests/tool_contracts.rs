use sentia_protocol::{
    tools::{ToolProvenanceSource, ToolResultStatus},
    ContractError, ContractErrorCode, ToolEvidence, ToolEvidenceKind, ToolName, ToolProvenance,
    ToolRequest, ToolResult, MAX_TOOL_INPUT_BYTES_V1, MAX_TOOL_OUTPUT_BYTES_V1,
};
use serde_json::json;

fn provenance() -> ToolProvenance {
    ToolProvenance {
        source: ToolProvenanceSource::Router,
        timestamp_ms: 100,
        request_id: None,
    }
}

#[test]
fn tool_request_accepts_generic_json_args() {
    let request = ToolRequest {
        version: "sentia.v1".to_owned(),
        request_id: "tool-req-1".try_into().expect("bounded request id"),
        name: ToolName::AptSearch,
        args: json!({
            "query": "curl",
            "limit": 5,
            "include": ["description", "package"]
        }),
        max_input_bytes: MAX_TOOL_INPUT_BYTES_V1 as u32,
        provenance: provenance(),
    };

    request.validate().expect("tool request should validate");
}

#[test]
fn tool_request_rejects_args_past_declared_bound() {
    let request = ToolRequest {
        version: "sentia.v1".to_owned(),
        request_id: "tool-req-2".try_into().expect("bounded request id"),
        name: ToolName::DirectoryContents,
        args: json!({ "path": "x".repeat(300) }),
        max_input_bytes: 32,
        provenance: provenance(),
    };

    let error = request
        .validate()
        .expect_err("tool request must reject oversized args");
    assert_eq!(error.code, ContractErrorCode::PayloadTooLarge);
}

#[test]
fn tool_result_requires_error_for_non_ok_status() {
    let result = ToolResult {
        version: "sentia.v1".to_owned(),
        request_id: "tool-req-3".try_into().expect("bounded request id"),
        name: ToolName::AptInstall,
        status: ToolResultStatus::PermissionDenied,
        data: json!({ "attempted": "vim" }),
        evidence: vec![],
        error: None,
        max_output_bytes: MAX_TOOL_OUTPUT_BYTES_V1 as u32,
        duration_ms: 22,
        truncated: false,
    };

    let error = result
        .validate()
        .expect_err("non-ok status must carry structured error");
    assert_eq!(error.code, ContractErrorCode::ValidationFailed);
}

#[test]
fn tool_result_round_trips_with_dynamic_data_and_evidence() {
    let result = ToolResult {
        version: "sentia.v1".to_owned(),
        request_id: "tool-req-4".try_into().expect("bounded request id"),
        name: ToolName::CpuStatus,
        status: ToolResultStatus::Ok,
        data: json!({
            "cores": 8,
            "load": [0.5, 0.7, 0.9],
            "notes": {"governor": "schedutil"}
        }),
        evidence: vec![ToolEvidence {
            kind: ToolEvidenceKind::Snapshot,
            path: "/run/user/1000/sentia/cpu.json".to_owned(),
            sha256: None,
        }],
        error: None,
        max_output_bytes: MAX_TOOL_OUTPUT_BYTES_V1 as u32,
        duration_ms: 5,
        truncated: false,
    };

    result.validate().expect("tool result should validate");
    let encoded = serde_json::to_value(&result).expect("encode tool result");
    assert!(encoded.get("data").is_some(), "tool result must expose `data`");
    assert!(encoded.get("result").is_none(), "legacy `result` field is not used");
}

#[test]
fn tool_result_rejects_oversized_payload() {
    let result = ToolResult {
        version: "sentia.v1".to_owned(),
        request_id: "tool-req-5".try_into().expect("bounded request id"),
        name: ToolName::JournalRecent,
        status: ToolResultStatus::Failed,
        data: json!({"log": "x".repeat(1024)}),
        evidence: vec![],
        error: Some(ContractError::new(
            ContractErrorCode::Internal,
            "fixture failure",
            true,
        )),
        max_output_bytes: 64,
        duration_ms: 9,
        truncated: false,
    };

    let error = result
        .validate()
        .expect_err("tool result must reject oversized data");
    assert_eq!(error.code, ContractErrorCode::PayloadTooLarge);
}
