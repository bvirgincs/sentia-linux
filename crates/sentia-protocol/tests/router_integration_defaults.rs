use sentia_protocol::{ContractErrorCode, RouterConfig};
use serde_json::{json, Value};

fn base_router_config_json() -> Value {
    json!({
        "version": "sentia.v1",
        "socket_path": "$XDG_RUNTIME_DIR/sentia/router.sock",
        "default_policy": "LOCAL_PREFERRED",
        "allow_remote_providers": true,
        "max_in_flight_requests": 16,
        "max_queue_depth": 64,
        "max_payload_bytes": 262144,
        "max_stream_event_bytes": 65536,
        "allowed_provider_ids": [],
        "integration_sockets": {
            "inference": {
                "role": "inference_private",
                "access_scope": "private_system",
                "path": "/run/sentia-inference/llama.sock",
                "owner_user": "sentia-inference",
                "directory_mode_octal": "0700",
                "socket_mode_octal": "0600",
                "require_peer_uid_check": true,
                "enforce_peer_uid_quotas": true,
                "nonsecret_metrics_only": false,
                "allow_cross_user_journal_access": false,
                "privileged_mutation_authority": false
            },
            "local_broker": {
                "role": "local_broker",
                "access_scope": "local_multi_user",
                "path": "/run/sentia-local/broker.sock",
                "directory_mode_octal": "0755",
                "socket_mode_octal": "0660",
                "require_peer_uid_check": true,
                "enforce_peer_uid_quotas": true,
                "nonsecret_metrics_only": false,
                "allow_cross_user_journal_access": false,
                "privileged_mutation_authority": false
            },
            "health_metrics": {
                "role": "health_metrics",
                "access_scope": "local_multi_user",
                "path": "/run/sentia-health/metrics.sock",
                "directory_mode_octal": "0755",
                "socket_mode_octal": "0660",
                "require_peer_uid_check": true,
                "enforce_peer_uid_quotas": true,
                "nonsecret_metrics_only": true,
                "allow_cross_user_journal_access": false,
                "privileged_mutation_authority": false
            },
            "user_router": {
                "role": "user_router",
                "access_scope": "per_user",
                "path": "$XDG_RUNTIME_DIR/sentia/router.sock",
                "directory_mode_octal": "0700",
                "socket_mode_octal": "0600",
                "require_peer_uid_check": true,
                "enforce_peer_uid_quotas": true,
                "nonsecret_metrics_only": false,
                "allow_cross_user_journal_access": false,
                "privileged_mutation_authority": false
            }
        }
    })
}

#[test]
fn integration_defaults_validate_without_deviation() {
    let config: RouterConfig =
        serde_json::from_value(base_router_config_json()).expect("valid config json");
    config.validate().expect("router config should validate");
    assert!(
        config.integration_sockets.deviations().is_empty(),
        "default integration sockets should not report deviations"
    );
}

#[test]
fn health_metrics_rejects_cross_user_journal_access() {
    let mut config = base_router_config_json();
    config["integration_sockets"]["health_metrics"]["allow_cross_user_journal_access"] =
        json!(true);
    let config: RouterConfig = serde_json::from_value(config).expect("valid config json shape");
    let error = config
        .validate()
        .expect_err("health metrics must reject unrestricted journal exposure");
    assert_eq!(error.code, ContractErrorCode::ValidationFailed);
}

#[test]
fn user_router_mode_deviation_requires_reason() {
    let mut config = base_router_config_json();
    config["integration_sockets"]["user_router"]["socket_mode_octal"] = json!("0660");
    let config: RouterConfig = serde_json::from_value(config).expect("valid config json shape");
    let error = config
        .validate()
        .expect_err("router socket mode deviation must include reason");
    assert_eq!(error.code, ContractErrorCode::ValidationFailed);
}

#[test]
fn user_router_mode_deviation_with_reason_is_allowed() {
    let mut config = base_router_config_json();
    config["integration_sockets"]["user_router"]["socket_mode_octal"] = json!("0660");
    config["integration_sockets"]["user_router"]["deviation_reason"] =
        json!("source-validated sandbox policy requires broader mode");
    let config: RouterConfig = serde_json::from_value(config).expect("valid config json shape");
    config
        .validate()
        .expect("documented source-validated deviation should pass contract checks");
}

#[test]
fn inference_endpoint_cannot_grant_mutation_authority() {
    let mut config = base_router_config_json();
    config["integration_sockets"]["inference"]["privileged_mutation_authority"] = json!(true);
    let config: RouterConfig = serde_json::from_value(config).expect("valid config json shape");
    let error = config
        .validate()
        .expect_err("inference admission must not grant privileged mutation authorization");
    assert_eq!(error.code, ContractErrorCode::ValidationFailed);
}
