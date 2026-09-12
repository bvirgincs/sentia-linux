use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ToolError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetDeviceStats {
    pub name: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_packets: u64,
    pub tx_packets: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransportProtocol {
    Tcp,
    Tcp6,
    Udp,
    Udp6,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SocketEntry {
    pub protocol: TransportProtocol,
    pub local_address: String,
    pub local_port: u16,
    pub state_hex: String,
    pub inode: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RouteEntry {
    pub interface: String,
    pub destination: String,
    pub gateway: String,
    pub flags_hex: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcStat {
    pub pid: i32,
    pub name: String,
    pub state: String,
    pub ppid: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CpuSummary {
    pub logical_cores: usize,
    pub model_name: Option<String>,
    pub vendor_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemSummary {
    pub mem_total_kb: Option<u64>,
    pub swap_total_kb: Option<u64>,
}

pub fn parse_proc_net_dev(contents: &str) -> Result<Vec<NetDeviceStats>, ToolError> {
    let mut stats = Vec::new();

    for line in contents.lines().skip(2) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let mut split = trimmed.splitn(2, ':');
        let interface = split
            .next()
            .ok_or_else(|| ToolError::parse("/proc/net/dev", "missing interface"))?
            .trim();
        let metrics = split
            .next()
            .ok_or_else(|| ToolError::parse("/proc/net/dev", "missing metrics"))?;

        let columns: Vec<&str> = metrics.split_whitespace().collect();
        if columns.len() < 16 {
            return Err(ToolError::parse(
                "/proc/net/dev",
                format!("expected >=16 columns, got {}", columns.len()),
            ));
        }

        let rx_bytes = parse_u64(columns[0], "/proc/net/dev rx_bytes")?;
        let rx_packets = parse_u64(columns[1], "/proc/net/dev rx_packets")?;
        let tx_bytes = parse_u64(columns[8], "/proc/net/dev tx_bytes")?;
        let tx_packets = parse_u64(columns[9], "/proc/net/dev tx_packets")?;

        stats.push(NetDeviceStats {
            name: interface.to_string(),
            rx_bytes,
            tx_bytes,
            rx_packets,
            tx_packets,
        });
    }

    stats.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(stats)
}

pub fn parse_proc_net_route(contents: &str) -> Result<Vec<RouteEntry>, ToolError> {
    let mut routes = Vec::new();

    for line in contents.lines().skip(1) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let columns: Vec<&str> = trimmed.split_whitespace().collect();
        if columns.len() < 4 {
            return Err(ToolError::parse(
                "/proc/net/route",
                format!("expected >=4 columns, got {}", columns.len()),
            ));
        }

        routes.push(RouteEntry {
            interface: columns[0].to_string(),
            destination: decode_proc_ipv4(columns[1])
                .unwrap_or_else(|| format!("hex:{}", columns[1])),
            gateway: decode_proc_ipv4(columns[2]).unwrap_or_else(|| format!("hex:{}", columns[2])),
            flags_hex: columns[3].to_string(),
        });
    }

    Ok(routes)
}

pub fn parse_proc_net_socket_table(
    contents: &str,
    protocol: TransportProtocol,
) -> Result<Vec<SocketEntry>, ToolError> {
    let mut sockets = Vec::new();

    for line in contents.lines().skip(1) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let columns: Vec<&str> = trimmed.split_whitespace().collect();
        if columns.len() < 10 {
            return Err(ToolError::parse(
                "/proc/net/*",
                format!("expected >=10 columns, got {}", columns.len()),
            ));
        }

        let local = columns[1];
        let state_hex = columns[3].to_string();
        let inode = parse_u64(columns[9], "inode").ok();

        let (local_address_hex, local_port_hex) = local
            .split_once(':')
            .ok_or_else(|| ToolError::parse("/proc/net/*", "socket missing local host:port"))?;

        let local_port = u16::from_str_radix(local_port_hex, 16)
            .map_err(|err| ToolError::parse("/proc/net/*", format!("invalid local port: {err}")))?;

        let local_address = match protocol {
            TransportProtocol::Tcp | TransportProtocol::Udp => decode_proc_ipv4(local_address_hex)
                .unwrap_or_else(|| format!("hex:{local_address_hex}")),
            TransportProtocol::Tcp6 | TransportProtocol::Udp6 => decode_proc_ipv6(local_address_hex)
                .unwrap_or_else(|| format!("hex:{local_address_hex}")),
        };

        sockets.push(SocketEntry {
            protocol,
            local_address,
            local_port,
            state_hex,
            inode,
        });
    }

    Ok(sockets)
}

pub fn parse_proc_stat_line(line: &str) -> Result<ProcStat, ToolError> {
    let open = line
        .find('(')
        .ok_or_else(|| ToolError::parse("/proc/<pid>/stat", "missing open parenthesis"))?;
    let close = line
        .rfind(')')
        .ok_or_else(|| ToolError::parse("/proc/<pid>/stat", "missing close parenthesis"))?;

    if close <= open {
        return Err(ToolError::parse(
            "/proc/<pid>/stat",
            "invalid command name segment",
        ));
    }

    let pid_segment = line[..open].trim();
    let pid = pid_segment
        .parse::<i32>()
        .map_err(|err| ToolError::parse("/proc/<pid>/stat", format!("invalid pid: {err}")))?;

    let name = line[(open + 1)..close].to_string();
    let after = line[(close + 1)..].trim();
    let mut fields = after.split_whitespace();

    let state = fields
        .next()
        .ok_or_else(|| ToolError::parse("/proc/<pid>/stat", "missing process state"))?
        .to_string();
    let ppid = fields
        .next()
        .ok_or_else(|| ToolError::parse("/proc/<pid>/stat", "missing parent pid"))?
        .parse::<i32>()
        .map_err(|err| ToolError::parse("/proc/<pid>/stat", format!("invalid ppid: {err}")))?;

    Ok(ProcStat {
        pid,
        name,
        state,
        ppid,
    })
}

pub fn parse_proc_status_map(contents: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for line in contents.lines() {
        if let Some((key, value)) = line.split_once(':') {
            map.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    map
}

pub fn parse_proc_cpuinfo(contents: &str) -> CpuSummary {
    let mut logical_cores = 0;
    let mut model_name = None;
    let mut vendor_id = None;

    for line in contents.lines() {
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            let value = value.trim().to_string();
            match key {
                "processor" => logical_cores += 1,
                "model name" if model_name.is_none() => model_name = Some(value),
                "vendor_id" if vendor_id.is_none() => vendor_id = Some(value),
                _ => {}
            }
        }
    }

    CpuSummary {
        logical_cores,
        model_name,
        vendor_id,
    }
}

pub fn parse_proc_meminfo(contents: &str) -> MemSummary {
    let mut mem_total_kb = None;
    let mut swap_total_kb = None;

    for line in contents.lines() {
        if let Some((key, value)) = line.split_once(':') {
            let value = value.trim();
            if key.trim() == "MemTotal" {
                mem_total_kb = parse_kb_field(value);
            } else if key.trim() == "SwapTotal" {
                swap_total_kb = parse_kb_field(value);
            }
        }
    }

    MemSummary {
        mem_total_kb,
        swap_total_kb,
    }
}

pub fn parse_os_release(contents: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();

    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if let Some((k, v)) = trimmed.split_once('=') {
            let value = v.trim_matches('"').to_string();
            map.insert(k.trim().to_string(), value);
        }
    }

    map
}

fn parse_kb_field(raw: &str) -> Option<u64> {
    let mut parts = raw.split_whitespace();
    parts.next().and_then(|num| num.parse::<u64>().ok())
}

fn parse_u64(value: &str, source: &str) -> Result<u64, ToolError> {
    value
        .parse::<u64>()
        .map_err(|err| ToolError::parse(source, err.to_string()))
}

fn decode_proc_ipv4(hex: &str) -> Option<String> {
    if hex.len() != 8 {
        return None;
    }

    let raw = u32::from_str_radix(hex, 16).ok()?;
    let bytes = raw.to_le_bytes();
    Some(format!(
        "{}.{}.{}.{}",
        bytes[0], bytes[1], bytes[2], bytes[3]
    ))
}

fn decode_proc_ipv6(hex: &str) -> Option<String> {
    if hex.len() != 32 {
        return None;
    }

    let mut bytes = [0_u8; 16];
    for idx in 0..16 {
        let start = idx * 2;
        let end = start + 2;
        let chunk = &hex[start..end];
        bytes[idx] = u8::from_str_radix(chunk, 16).ok()?;
    }

    Some(std::net::Ipv6Addr::from(bytes).to_string())
}
