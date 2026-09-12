use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::parsers::{
    parse_proc_net_dev, parse_proc_net_route, parse_proc_net_socket_table, NetDeviceStats, RouteEntry,
    SocketEntry, TransportProtocol,
};
use crate::result::{PrivacyClass, Provenance, ToolResult};
use crate::validation::{
    bounded_limit, read_text_file_bounded, DEFAULT_LIST_LIMIT, MAX_LIST_LIMIT,
};
use crate::ToolError;

const PROC_NET_DEV: &str = "/proc/net/dev";
const PROC_NET_ROUTE: &str = "/proc/net/route";
const PROC_NET_TCP: &str = "/proc/net/tcp";
const PROC_NET_TCP6: &str = "/proc/net/tcp6";
const PROC_NET_UDP: &str = "/proc/net/udp";
const PROC_NET_UDP6: &str = "/proc/net/udp6";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkStatusInput;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkInterfacesInput;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListeningPortsInput {
    pub limit: Option<usize>,
    #[serde(default)]
    pub include_udp: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkInterfaceSummary {
    pub name: String,
    pub operstate: Option<String>,
    pub mac_address: Option<String>,
    pub mtu: Option<u64>,
    pub flags_hex: Option<String>,
    pub rx_bytes: Option<u64>,
    pub tx_bytes: Option<u64>,
}

pub fn network_status(_input: NetworkStatusInput) -> Result<ToolResult, ToolError> {
    let interfaces = collect_interfaces()?;
    let routes = collect_routes()?;
    let listening = collect_listening_ports_internal(true)?;

    let interfaces_up = interfaces
        .iter()
        .filter(|iface| iface.operstate.as_deref() == Some("up"))
        .count();

    let default_routes = routes
        .iter()
        .filter(|route| route.destination == "0.0.0.0")
        .count();

    let tcp_listening = listening
        .iter()
        .filter(|entry| matches!(entry.protocol, TransportProtocol::Tcp | TransportProtocol::Tcp6))
        .count();
    let udp_bound = listening
        .iter()
        .filter(|entry| matches!(entry.protocol, TransportProtocol::Udp | TransportProtocol::Udp6))
        .count();

    Ok(ToolResult::new(
        "network_status",
        PrivacyClass::SystemMetadata,
        Duration::from_secs(2),
        vec![
            Provenance {
                source: "procfs".to_string(),
                detail: format!("{}, {}, {}, {}", PROC_NET_DEV, PROC_NET_ROUTE, PROC_NET_TCP, PROC_NET_TCP6),
            },
            Provenance {
                source: "sysfs".to_string(),
                detail: "/sys/class/net/*".to_string(),
            },
        ],
        false,
        json!({
            "interface_count": interfaces.len(),
            "interfaces_up": interfaces_up,
            "default_route_count": default_routes,
            "tcp_listening_ports": tcp_listening,
            "udp_bound_ports": udp_bound,
            "routes": routes,
        }),
    ))
}

pub fn network_interfaces(_input: NetworkInterfacesInput) -> Result<ToolResult, ToolError> {
    let interfaces = collect_interfaces()?;

    Ok(ToolResult::new(
        "network_interfaces",
        PrivacyClass::SystemMetadata,
        Duration::from_secs(2),
        vec![
            Provenance {
                source: "procfs".to_string(),
                detail: PROC_NET_DEV.to_string(),
            },
            Provenance {
                source: "sysfs".to_string(),
                detail: "/sys/class/net/*".to_string(),
            },
        ],
        false,
        json!({
            "interfaces": interfaces,
        }),
    ))
}

pub fn listening_ports(input: ListeningPortsInput) -> Result<ToolResult, ToolError> {
    let limit = bounded_limit(input.limit, DEFAULT_LIST_LIMIT, MAX_LIST_LIMIT, "limit")?;

    let mut entries = collect_listening_ports_internal(input.include_udp)?;
    entries.sort_by(|a, b| {
        a.local_port
            .cmp(&b.local_port)
            .then_with(|| a.local_address.cmp(&b.local_address))
    });

    let partial = entries.len() > limit;
    if entries.len() > limit {
        entries.truncate(limit);
    }

    Ok(ToolResult::new(
        "listening_ports",
        PrivacyClass::SystemMetadata,
        Duration::from_secs(2),
        vec![Provenance {
            source: "procfs".to_string(),
            detail: if input.include_udp {
                format!("{PROC_NET_TCP}, {PROC_NET_TCP6}, {PROC_NET_UDP}, {PROC_NET_UDP6}")
            } else {
                format!("{PROC_NET_TCP}, {PROC_NET_TCP6}")
            },
        }],
        partial,
        json!({
            "entries": entries,
            "include_udp": input.include_udp,
            "limit": limit,
        }),
    ))
}

fn collect_interfaces() -> Result<Vec<NetworkInterfaceSummary>, ToolError> {
    let net_dev = read_text_file_bounded(Path::new(PROC_NET_DEV), 512 * 1024)?;
    let dev_stats = parse_proc_net_dev(&net_dev)?;
    let stats_by_name: BTreeMap<String, NetDeviceStats> = dev_stats
        .into_iter()
        .map(|stat| (stat.name.clone(), stat))
        .collect();

    let mut interfaces = Vec::new();

    let read_dir = fs::read_dir("/sys/class/net")
        .map_err(|err| ToolError::io("reading /sys/class/net", err.to_string()))?;

    for entry in read_dir {
        let entry = entry.map_err(|err| ToolError::io("iterating /sys/class/net", err.to_string()))?;
        let name = entry.file_name().to_string_lossy().to_string();
        let base = entry.path();

        let operstate = read_trimmed_optional(&base.join("operstate"));
        let mac_address = read_trimmed_optional(&base.join("address"));
        let mtu = read_trimmed_optional(&base.join("mtu")).and_then(|value| value.parse::<u64>().ok());
        let flags_hex = read_trimmed_optional(&base.join("flags")).map(|value| value.trim_start_matches("0x").to_string());

        let stats = stats_by_name.get(&name);

        interfaces.push(NetworkInterfaceSummary {
            name,
            operstate,
            mac_address,
            mtu,
            flags_hex,
            rx_bytes: stats.map(|stat| stat.rx_bytes),
            tx_bytes: stats.map(|stat| stat.tx_bytes),
        });
    }

    interfaces.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(interfaces)
}

fn collect_routes() -> Result<Vec<RouteEntry>, ToolError> {
    let route_text = read_text_file_bounded(Path::new(PROC_NET_ROUTE), 256 * 1024)?;
    parse_proc_net_route(&route_text)
}

fn collect_listening_ports_internal(include_udp: bool) -> Result<Vec<SocketEntry>, ToolError> {
    let tcp = read_and_parse_socket_file(PROC_NET_TCP, TransportProtocol::Tcp, true)?;
    let tcp6 = read_and_parse_socket_file(PROC_NET_TCP6, TransportProtocol::Tcp6, true)?;

    let mut result = Vec::new();
    result.extend(tcp);
    result.extend(tcp6);

    if include_udp {
        let udp = read_and_parse_socket_file(PROC_NET_UDP, TransportProtocol::Udp, false)?;
        let udp6 = read_and_parse_socket_file(PROC_NET_UDP6, TransportProtocol::Udp6, false)?;
        result.extend(udp);
        result.extend(udp6);
    }

    Ok(result)
}

fn read_and_parse_socket_file(
    path: &str,
    protocol: TransportProtocol,
    require_tcp_listen: bool,
) -> Result<Vec<SocketEntry>, ToolError> {
    let content = read_text_file_bounded(Path::new(path), 512 * 1024)?;
    let mut entries = parse_proc_net_socket_table(&content, protocol)?;

    entries.retain(|entry| {
        if entry.local_port == 0 {
            return false;
        }

        if require_tcp_listen {
            entry.state_hex == "0A"
        } else {
            true
        }
    });

    Ok(entries)
}

fn read_trimmed_optional(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
