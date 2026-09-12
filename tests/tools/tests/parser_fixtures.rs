use std::fs;
use std::path::Path;

use sentia_tools::parsers::{
    parse_os_release, parse_proc_cpuinfo, parse_proc_meminfo, parse_proc_net_dev,
    parse_proc_net_route, parse_proc_net_socket_table, TransportProtocol,
};

fn fixture(name: &str) -> String {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    fs::read_to_string(base.join(name)).expect("fixture must be readable")
}

#[test]
fn parses_proc_net_dev_fixture() {
    let parsed = parse_proc_net_dev(&fixture("proc_net_dev.txt")).expect("parse /proc/net/dev");

    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].name, "eth0");
    assert_eq!(parsed[0].rx_bytes, 9_876_543);
    assert_eq!(parsed[1].name, "lo");
    assert_eq!(parsed[1].tx_packets, 1_200);
}

#[test]
fn parses_proc_net_tcp_fixture() {
    let parsed = parse_proc_net_socket_table(&fixture("proc_net_tcp.txt"), TransportProtocol::Tcp)
        .expect("parse /proc/net/tcp");

    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].local_address, "127.0.0.1");
    assert_eq!(parsed[0].local_port, 22);
    assert_eq!(parsed[0].state_hex, "0A");
    assert_eq!(parsed[1].state_hex, "01");
}

#[test]
fn parses_proc_net_route_fixture() {
    let parsed = parse_proc_net_route(&fixture("proc_net_route.txt")).expect("parse /proc/net/route");

    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].interface, "eth0");
    assert_eq!(parsed[0].destination, "0.0.0.0");
    assert_eq!(parsed[0].gateway, "192.168.2.1");
}

#[test]
fn parses_cpu_mem_and_os_fixtures() {
    let cpu = parse_proc_cpuinfo(&fixture("proc_cpuinfo.txt"));
    assert_eq!(cpu.logical_cores, 2);
    assert_eq!(cpu.model_name.as_deref(), Some("Example CPU 2.50GHz"));

    let mem = parse_proc_meminfo(&fixture("proc_meminfo.txt"));
    assert_eq!(mem.mem_total_kb, Some(16_384_256));
    assert_eq!(mem.swap_total_kb, Some(2_097_148));

    let os = parse_os_release(&fixture("os_release.txt"));
    assert_eq!(os.get("ID").map(String::as_str), Some("debian"));
    assert_eq!(os.get("VERSION_CODENAME").map(String::as_str), Some("trixie"));
}
