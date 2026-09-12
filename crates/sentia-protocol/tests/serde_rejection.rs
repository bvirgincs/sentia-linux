use sentia_protocol::router::RouterRequest;
use serde_json::json;

fn valid_request_json() -> serde_json::Value {
    json!({
        "version": "sentia.v1",
        "request_id": "req-1",
        "session_id": "session-1",
        "created_at_ms": 1,
        "capability": "inference",
        "policy": "LOCAL_ONLY",
        "provenance": {
            "source": "user_input",
            "sensitivity": "public",
            "redaction_applied": false,
            "origin_label": "cli"
        },
        "payload": {
            "mime_type": "text/plain",
            "content": "hello",
            "byte_count": 5,
            "truncated": false
        }
    })
}

#[test]
fn rejects_unknown_policy_variant() {
    let mut request = valid_request_json();
    request["policy"] = json!("REMOTE_ALWAYS");

    let result = serde_json::from_value::<RouterRequest>(request);
    assert!(result.is_err(), "unexpected success for unknown policy");
}

#[test]
fn rejects_oversized_request_id() {
    let mut request = valid_request_json();
    request["request_id"] = json!(String::from("r").repeat(65));

    let result = serde_json::from_value::<RouterRequest>(request);
    assert!(result.is_err(), "unexpected success for oversized request_id");
}
