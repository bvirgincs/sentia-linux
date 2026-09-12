use sentia_protocol::{
    router::RouterRequest,
    ContractErrorCode, ToolName, ToolRegistry,
};
use serde_json::json;

fn ask_before_remote_without_consent() -> RouterRequest {
    serde_json::from_value(json!({
        "version": "sentia.v1",
        "request_id": "req-ask-1",
        "session_id": "session-1",
        "created_at_ms": 10,
        "capability": "inference",
        "policy": "ASK_BEFORE_REMOTE",
        "provenance": {
            "source": "user_input",
            "sensitivity": "sensitive",
            "redaction_applied": true,
            "origin_label": "terminal_capture"
        },
        "payload": {
            "mime_type": "text/plain",
            "content": "what is on disk",
            "byte_count": 15,
            "truncated": false
        }
    }))
    .expect("request JSON should deserialize")
}

#[test]
fn ask_before_remote_requires_consent_token() {
    let request = ask_before_remote_without_consent();
    let error = request
        .validate()
        .expect_err("validation should fail without consent token");
    assert_eq!(error.code, ContractErrorCode::PolicyDenied);
}

#[test]
fn required_registry_contains_all_tools_without_unsafe_names() {
    let registry = ToolRegistry::required_v1();
    registry
        .validate_complete_required_set()
        .expect("required registry must validate");
    assert_eq!(registry.tools.len(), ToolName::ALL.len());

    assert!(
        registry
            .tools
            .iter()
            .all(|tool| tool.name.as_str() != "execute_shell"),
        "registry must not expose execute_shell"
    );
    assert!(
        registry
            .tools
            .iter()
            .all(|tool| tool.name.as_str() != "client_approved_authority"),
        "registry must not trust client-approved authority flags"
    );
    assert!(
        registry.tools.iter().all(|tool| {
            tool.input_schema_ref
                == "schemas/tools/tool-invocation-v1.schema.json#/$defs/tool_request"
        }),
        "all tools should use shared ToolRequest schema"
    );
    assert!(
        registry.tools.iter().all(|tool| {
            tool.output_schema_ref
                == "schemas/tools/tool-invocation-v1.schema.json#/$defs/tool_result"
        }),
        "all tools should use shared ToolResult schema"
    );
}
