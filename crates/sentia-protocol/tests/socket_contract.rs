use sentia_protocol::socket::JsonlFrame;
use serde_json::json;

#[test]
fn cancel_frame_round_trips_with_ids_and_version() {
    let frame = json!({
        "frame_type": "cancel",
        "version": "sentia.v1",
        "frame_id": "frame-2",
        "stream_id": "stream-1",
        "cancel": {
            "cancel_id": "cancel-1",
            "request_id": "req-9",
            "reason": "user_requested",
            "requested_at_ms": 42
        }
    });

    let parsed = serde_json::from_value::<JsonlFrame>(frame).expect("deserialize cancel frame");
    parsed.validate_version().expect("version validation");

    let encoded = serde_json::to_value(parsed).expect("serialize cancel frame");
    assert_eq!(encoded["frame_type"], "cancel");
    assert_eq!(encoded["cancel"]["cancel_id"], "cancel-1");
}

#[test]
fn tool_request_frame_round_trips_with_name_and_args() {
    let frame = json!({
        "frame_type": "tool_request",
        "version": "sentia.v1",
        "frame_id": "frame-3",
        "stream_id": "stream-2",
        "tool_request": {
            "version": "sentia.v1",
            "request_id": "req-tool-1",
            "name": "apt_search",
            "args": { "query": "vim", "limit": 3 },
            "max_input_bytes": 4096,
            "provenance": {
                "source": "router",
                "timestamp_ms": 123
            }
        }
    });

    let parsed = serde_json::from_value::<JsonlFrame>(frame).expect("deserialize tool request frame");
    parsed.validate_version().expect("version validation");

    let encoded = serde_json::to_value(parsed).expect("serialize tool request frame");
    assert_eq!(encoded["frame_type"], "tool_request");
    assert_eq!(encoded["tool_request"]["name"], "apt_search");
    assert!(encoded["tool_request"].get("args").is_some());
}
