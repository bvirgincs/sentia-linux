use sentia_health::{AlertEngine, HealthSnapshot};
use sentia_health::schema::{FilesystemSample, FilesystemStatus, MemoryPressure};

fn base_snapshot(now: u64) -> HealthSnapshot {
    HealthSnapshot::empty(now)
}

#[test]
fn cpu_alert_uses_hysteresis_and_dedup() {
    let mut engine = AlertEngine::new(16);

    let mut first = base_snapshot(1_000);
    first.cpu.utilization_pct = 95.0;
    let (active_first, log_first) = engine.evaluate(&first);
    assert!(active_first.is_empty());
    assert!(log_first.is_empty());

    let mut second = base_snapshot(2_000);
    second.cpu.utilization_pct = 95.0;
    let (active_second, log_second) = engine.evaluate(&second);
    assert_eq!(active_second.len(), 1);
    assert_eq!(active_second[0].id, "cpu_high");
    assert_eq!(log_second.len(), 1);

    let mut third = base_snapshot(3_000);
    third.cpu.utilization_pct = 95.5;
    let (_active_third, log_third) = engine.evaluate(&third);
    assert_eq!(log_third.len(), 1, "insignificant change should not emit");

    let mut fourth = base_snapshot(4_000);
    fourth.cpu.utilization_pct = 60.0;
    let (active_fourth, _) = engine.evaluate(&fourth);
    assert_eq!(active_fourth.len(), 1, "clear requires 2 samples");

    let mut fifth = base_snapshot(5_000);
    fifth.cpu.utilization_pct = 60.0;
    let (active_fifth, log_fifth) = engine.evaluate(&fifth);
    assert!(active_fifth.is_empty());
    assert_eq!(log_fifth.len(), 2, "activation and resolution records");
}

#[test]
fn filesystem_full_triggers_immediate_alert() {
    let mut engine = AlertEngine::new(8);

    let mut snapshot = base_snapshot(10_000);
    snapshot.filesystem = FilesystemStatus {
        filesystems: vec![FilesystemSample {
            mount_point: "/".to_string(),
            fs_type: "ext4".to_string(),
            total_bytes: 100,
            used_bytes: 100,
            available_bytes: 0,
            used_pct: 100.0,
            full: true,
        }],
        full_filesystems: 1,
    };

    let (active, log) = engine.evaluate(&snapshot);
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].id, "filesystem_full");
    assert_eq!(log.len(), 1);
}

#[test]
fn pressure_and_process_churn_need_sustained_samples() {
    let mut engine = AlertEngine::new(16);

    let mut first = base_snapshot(20_000);
    first.memory.pressure = Some(MemoryPressure {
        some_avg10: 1.4,
        some_avg60: 1.0,
        some_avg300: 0.5,
        full_avg10: 0.1,
        full_avg60: 0.1,
        full_avg300: 0.1,
    });
    first.processes.churned_processes = 70;
    let (active_first, _) = engine.evaluate(&first);
    assert!(active_first.is_empty());

    let mut second = first.clone();
    second.collected_at_unix_ms = 21_000;
    let (active_second, _) = engine.evaluate(&second);
    assert_eq!(active_second.len(), 1, "process churn should trigger after 2 samples");
    assert_eq!(active_second[0].id, "process_churn");

    let mut third = second.clone();
    third.collected_at_unix_ms = 22_000;
    let (active_third, _) = engine.evaluate(&third);
    assert!(active_third.iter().any(|a| a.id == "memory_pressure"));
}
