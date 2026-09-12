use sentia_tools::protocol::{
    handle_request, readonly_tool_names, ToolErrorCategory, ToolRequest, ToolResponse,
    READ_ONLY_PROTOCOL_VERSION,
};

#[test]
fn list_framing_exposes_only_read_only_tools() {
    let response = handle_request(ToolRequest::List {
        protocol_version: READ_ONLY_PROTOCOL_VERSION.to_string(),
        request_id: Some("req-1".to_string()),
    });

    let expected_names = readonly_tool_names();

    match response {
        ToolResponse::List {
            protocol_version,
            request_id,
            tools,
        } => {
            assert_eq!(protocol_version, READ_ONLY_PROTOCOL_VERSION);
            assert_eq!(request_id.as_deref(), Some("req-1"));
            assert_eq!(tools.len(), expected_names.len());

            let actual_names = tools.iter().map(|tool| tool.name.as_str()).collect::<Vec<_>>();
            assert_eq!(actual_names, expected_names);

            assert!(tools.iter().all(|tool| tool.read_only));
            assert!(tools.iter().all(|tool| !tool.supports_cancellation));
            assert!(tools.iter().all(|tool| tool.input_schema.is_object()));
            assert!(!actual_names.contains(&"apt_install"));
            assert!(!actual_names.contains(&"service_restart"));
            assert!(!actual_names.contains(&"process_kill"));
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[test]
fn invoke_framing_routes_to_tool_implementation() {
    let response = handle_request(ToolRequest::Invoke {
        protocol_version: READ_ONLY_PROTOCOL_VERSION.to_string(),
        request_id: Some("req-2".to_string()),
        call_id: "call-1".to_string(),
        tool_name: "command_exists".to_string(),
        input: serde_json::json!({ "command": "ls" }),
    });

    match response {
        ToolResponse::Result {
            protocol_version,
            request_id,
            call_id,
            result,
        } => {
            assert_eq!(protocol_version, READ_ONLY_PROTOCOL_VERSION);
            assert_eq!(request_id.as_deref(), Some("req-2"));
            assert_eq!(call_id, "call-1");
            assert_eq!(result.tool_name, "command_exists");
            assert!(result.data["exists"].as_bool().is_some());
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[test]
fn invoke_unknown_tool_returns_not_found_error() {
    let response = handle_request(ToolRequest::Invoke {
        protocol_version: READ_ONLY_PROTOCOL_VERSION.to_string(),
        request_id: None,
        call_id: "call-unknown".to_string(),
        tool_name: "apt_install".to_string(),
        input: serde_json::json!({}),
    });

    match response {
        ToolResponse::Error { call_id, error, .. } => {
            assert_eq!(call_id.as_deref(), Some("call-unknown"));
            assert_eq!(error.category, ToolErrorCategory::NotFound);
            assert!(!error.retryable);
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[test]
fn invoke_privileged_apply_is_rejected() {
    let response = handle_request(ToolRequest::Invoke {
        protocol_version: READ_ONLY_PROTOCOL_VERSION.to_string(),
        request_id: None,
        call_id: "call-apply".to_string(),
        tool_name: "org.sentia.System1.Apply".to_string(),
        input: serde_json::json!({
            "plan_id": "abc",
            "digest": "def"
        }),
    });

    match response {
        ToolResponse::Error { call_id, error, .. } => {
            assert_eq!(call_id.as_deref(), Some("call-apply"));
            assert_eq!(error.category, ToolErrorCategory::PermissionDenied);
            assert!(!error.retryable);
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[test]
fn cancel_form_is_explicitly_not_supported_in_flight() {
    let response = handle_request(ToolRequest::Cancel {
        protocol_version: READ_ONLY_PROTOCOL_VERSION.to_string(),
        request_id: Some("req-3".to_string()),
        call_id: "call-2".to_string(),
    });

    match response {
        ToolResponse::Cancelled {
            protocol_version,
            request_id,
            call_id,
            accepted,
            reason,
        } => {
            assert_eq!(protocol_version, READ_ONLY_PROTOCOL_VERSION);
            assert_eq!(request_id.as_deref(), Some("req-3"));
            assert_eq!(call_id, "call-2");
            assert!(!accepted);
            assert!(reason.contains("cancel before dispatch"));
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[test]
fn framing_rejects_wrong_protocol_version() {
    let response = handle_request(ToolRequest::List {
        protocol_version: "sentia.tools.readonly.v0".to_string(),
        request_id: Some("req-4".to_string()),
    });

    match response {
        ToolResponse::Error {
            protocol_version,
            request_id,
            call_id,
            error,
        } => {
            assert_eq!(protocol_version, READ_ONLY_PROTOCOL_VERSION);
            assert_eq!(request_id.as_deref(), Some("req-4"));
            assert!(call_id.is_none());
            assert_eq!(error.category, ToolErrorCategory::InvalidInput);
            assert!(!error.retryable);
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[test]
fn request_serialization_uses_op_discriminator() {
    let value = serde_json::to_value(ToolRequest::Invoke {
        protocol_version: READ_ONLY_PROTOCOL_VERSION.to_string(),
        request_id: Some("req-5".to_string()),
        call_id: "call-3".to_string(),
        tool_name: "network_status".to_string(),
        input: serde_json::json!({}),
    })
    .expect("serialize request");

    assert_eq!(value["op"].as_str(), Some("invoke"));
    assert_eq!(value["tool_name"].as_str(), Some("network_status"));
}
