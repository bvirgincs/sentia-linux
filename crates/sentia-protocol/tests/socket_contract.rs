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
