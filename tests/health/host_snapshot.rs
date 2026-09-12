use std::path::PathBuf;

use sentia_health::{CollectorConfig, DaemonConfig, HealthCollector, HealthMonitor};
use sentia_health::schema::CapabilityState;

#[test]
fn host_snapshot_contains_required_sections_and_bounds() {
    let mut monitor = HealthMonitor::default();
    let snapshot = monitor.sample();
    assert_eq!(snapshot.elapsed_ms, 0, "first sample should have zero elapsed");
    assert_eq!(snapshot.network.total_rx_bps, 0.0);
    assert_eq!(snapshot.network.total_tx_bps, 0.0);
    assert_eq!(snapshot.disk.total_read_bps, 0.0);
    assert_eq!(snapshot.disk.total_write_bps, 0.0);

    let json = serde_json::to_value(&snapshot).expect("snapshot should serialize");
    for key in [
        "cpu",
        "memory",
        "swap",
        "filesystem",
        "disk",
        "diskhealth",
        "temperature",
        "tools",
    ] {
        assert!(json.get(key).is_some(), "missing top-level key: {key}");
    }

    assert!(snapshot.kernel.recent_errors.len() <= 16);
    for err in &snapshot.kernel.recent_errors {
        assert!(err.message.len() <= 180);
        assert!(!err.message.contains('\n'));
    }
}

#[test]
fn missing_temperature_fixture_reports_unavailable_capability() {
    let collector = HealthCollector::new(CollectorConfig::default());
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/health/fixtures/thermal-empty");
    let (temperature, capability) = collector.collect_temperature_from_root(&root);

    assert!(temperature.sensors.is_empty());
    assert_eq!(capability.state, CapabilityState::Unavailable);
}

#[test]
fn default_socket_path_matches_integration_contract() {
    let daemon = DaemonConfig::default();
    assert_eq!(
        daemon.socket_path,
        PathBuf::from("/run/sentia-health/metrics.sock")
    );
}
