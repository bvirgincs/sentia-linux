use sentia_health::procfs::{
    cpu_utilization_pct, parse_cpu_total, parse_diskstats, parse_meminfo, parse_memory_pressure,
    parse_net_dev, parse_process_stat, sanitize_log_message,
};

fn fixture(name: &str) -> String {
    let path = format!(
        "{}/../../tests/health/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(path).expect("fixture should load")
}

#[test]
fn cpu_delta_and_reset_are_handled() {
    let prev = parse_cpu_total(&fixture("proc_stat_prev.txt")).expect("prev cpu");
    let curr = parse_cpu_total(&fixture("proc_stat_curr.txt")).expect("curr cpu");

    let utilization = cpu_utilization_pct(curr, prev).expect("utilization");
    assert!(utilization > 54.0 && utilization < 55.0);

    let reset = parse_cpu_total(&fixture("proc_stat_reset.txt")).expect("reset cpu");
    assert!(cpu_utilization_pct(reset, curr).is_none());
}

#[test]
fn meminfo_and_psi_parse() {
    let mem = parse_meminfo(&fixture("meminfo.txt")).expect("meminfo");
    assert_eq!(mem.total_bytes, 1_024_000_000);
    assert_eq!(mem.swap_total_bytes, 204_800_000);

    let psi = parse_memory_pressure(&fixture("psi_memory.txt")).expect("psi");
    assert_eq!(psi.some_avg10, 1.2);
    assert_eq!(psi.full_avg60, 0.08);
}

#[test]
fn net_and_disk_parse() {
    let net = parse_net_dev(&fixture("net_dev.txt"));
    let eth0 = net.get("eth0").expect("eth0 present");
    assert_eq!(eth0.rx_bytes, 1_048_576);
    assert_eq!(eth0.tx_bytes, 2_097_152);

    let disks = parse_diskstats(&fixture("diskstats.txt"));
    assert!(disks.contains_key("sda"));
    assert!(disks.contains_key("nvme0n1"));
    assert!(!disks.contains_key("sda1"));
    assert!(!disks.contains_key("nvme0n1p1"));
}

#[test]
fn process_stat_and_redaction() {
    let proc = parse_process_stat(&fixture("process_stat.txt")).expect("process stat");
    assert_eq!(proc.pid, 1234);
    assert_eq!(proc.name, "python3");
    assert_eq!(proc.cpu_jiffies, 600);

    let redacted = sanitize_log_message(
        "token=abcdef password=hunter2 normal=ok giant=abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz",
        200,
    );
    assert!(redacted.contains("token=<redacted>"));
    assert!(redacted.contains("password=<redacted>"));
    assert!(redacted.contains("normal=ok"));
    assert!(!redacted.contains("abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz"));
}
