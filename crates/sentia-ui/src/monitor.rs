use serde::{Deserialize, Serialize};
use std::ffi::CString;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::transport::health_socket_path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CapabilityState {
    Available,
    Unsupported,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityValue<T> {
    pub state: CapabilityState,
    pub value: Option<T>,
    pub note: Option<String>,
}

impl<T> CapabilityValue<T> {
    pub fn available(value: T) -> Self {
        Self {
            state: CapabilityState::Available,
            value: Some(value),
            note: None,
        }
    }

    pub fn unsupported(note: impl Into<String>) -> Self {
        Self {
            state: CapabilityState::Unsupported,
            value: None,
            note: Some(note.into()),
        }
    }

    pub fn unavailable(note: impl Into<String>) -> Self {
        Self {
            state: CapabilityState::Unavailable,
            value: None,
            note: Some(note.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuLoad {
    pub one: f32,
    pub five: f32,
    pub fifteen: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryMetrics {
    pub total_kib: u64,
    pub available_kib: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapMetrics {
    pub total_kib: u64,
    pub free_kib: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapacityMetrics {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub used_percent: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkMetrics {
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthSnapshot {
    pub source: String,
    pub collected_at: String,
    pub cpu_load: CapabilityValue<CpuLoad>,
    pub memory: CapabilityValue<MemoryMetrics>,
    pub swap: CapabilityValue<SwapMetrics>,
    pub root_capacity: CapabilityValue<CapacityMetrics>,
    pub disk_health: CapabilityValue<String>,
    pub network: CapabilityValue<NetworkMetrics>,
    pub temperature_celsius: CapabilityValue<f32>,
}

impl HealthSnapshot {
    pub fn collect(socket_path: Option<&Path>) -> Self {
        let socket = socket_path
            .map(PathBuf::from)
            .unwrap_or_else(health_socket_path);

        match Self::from_health_socket(&socket) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let mut snapshot = Self::from_local_system();
                snapshot.source = format!("local-fallback ({error})");
                snapshot
            }
        }
    }

    pub fn from_health_socket(path: &Path) -> Result<Self, String> {
        let mut stream = UnixStream::connect(path)
            .map_err(|error| format!("failed to connect {}: {error}", path.display()))?;

        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|error| format!("failed to set read timeout: {error}"))?;
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .map_err(|error| format!("failed to set write timeout: {error}"))?;

        stream
            .write_all(b"{\"request\":\"snapshot\"}\n")
            .map_err(|error| format!("failed to request metrics: {error}"))?;

        let mut payload = String::new();
        stream
            .read_to_string(&mut payload)
            .map_err(|error| format!("failed to read metrics: {error}"))?;

        serde_json::from_str(payload.trim())
            .map_err(|error| format!("invalid metrics payload: {error}"))
    }

    pub fn from_local_system() -> Self {
        let (memory, swap) = read_mem_and_swap();

        Self {
            source: "local-procfs".to_string(),
            collected_at: now_timestamp(),
            cpu_load: read_loadavg(),
            memory,
            swap,
            root_capacity: read_root_capacity(),
            disk_health: CapabilityValue::unsupported(
                "SMART/NVMe health needs sentia-health privileged probe",
            ),
            network: read_network_totals(),
            temperature_celsius: read_temperature(),
        }
    }
}

fn now_timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    format!("unix:{now}")
}

fn read_loadavg() -> CapabilityValue<CpuLoad> {
    let Ok(raw) = fs::read_to_string("/proc/loadavg") else {
        return CapabilityValue::unavailable("/proc/loadavg unavailable");
    };

    let mut parts = raw.split_whitespace();
    let one = parts.next().and_then(|v| v.parse::<f32>().ok());
    let five = parts.next().and_then(|v| v.parse::<f32>().ok());
    let fifteen = parts.next().and_then(|v| v.parse::<f32>().ok());

    match (one, five, fifteen) {
        (Some(one), Some(five), Some(fifteen)) => CapabilityValue::available(CpuLoad {
            one,
            five,
            fifteen,
        }),
        _ => CapabilityValue::unavailable("load average parse failed"),
    }
}

fn read_mem_and_swap() -> (CapabilityValue<MemoryMetrics>, CapabilityValue<SwapMetrics>) {
    let Ok(raw) = fs::read_to_string("/proc/meminfo") else {
        return (
            CapabilityValue::unavailable("/proc/meminfo unavailable"),
            CapabilityValue::unavailable("/proc/meminfo unavailable"),
        );
    };

    let mut mem_total = None;
    let mut mem_available = None;
    let mut swap_total = None;
    let mut swap_free = None;

    for line in raw.lines() {
        let mut parts = line.split_whitespace();
        match parts.next().unwrap_or_default() {
            "MemTotal:" => mem_total = parts.next().and_then(|value| value.parse::<u64>().ok()),
            "MemAvailable:" => {
                mem_available = parts.next().and_then(|value| value.parse::<u64>().ok())
            }
            "SwapTotal:" => swap_total = parts.next().and_then(|value| value.parse::<u64>().ok()),
            "SwapFree:" => swap_free = parts.next().and_then(|value| value.parse::<u64>().ok()),
            _ => {}
        }
    }

    let memory = match (mem_total, mem_available) {
        (Some(total_kib), Some(available_kib)) => {
            CapabilityValue::available(MemoryMetrics { total_kib, available_kib })
        }
        _ => CapabilityValue::unavailable("memory fields missing"),
    };

    let swap = match (swap_total, swap_free) {
        (Some(total_kib), Some(free_kib)) => CapabilityValue::available(SwapMetrics {
            total_kib,
            free_kib,
        }),
        _ => CapabilityValue::unavailable("swap fields missing"),
    };

    (memory, swap)
}

fn read_root_capacity() -> CapabilityValue<CapacityMetrics> {
    let c_path = CString::new("/").expect("literal root path should be valid CString");
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();

    let rc = unsafe { libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) };
    if rc != 0 {
        return CapabilityValue::unavailable("statvfs failed");
    }

    let stat = unsafe { stat.assume_init() };
    let total_bytes = stat.f_blocks.saturating_mul(stat.f_frsize as u64);
    let available_bytes = stat.f_bavail.saturating_mul(stat.f_frsize as u64);
    let used_bytes = total_bytes.saturating_sub(available_bytes);
    let used_percent = if total_bytes == 0 {
        0.0
    } else {
        (used_bytes as f32 / total_bytes as f32) * 100.0
    };

    CapabilityValue::available(CapacityMetrics {
        total_bytes,
        used_bytes,
        used_percent,
    })
}

fn read_network_totals() -> CapabilityValue<NetworkMetrics> {
    let Ok(raw) = fs::read_to_string("/proc/net/dev") else {
        return CapabilityValue::unavailable("/proc/net/dev unavailable");
    };

    let mut rx_total = 0u64;
    let mut tx_total = 0u64;

    for line in raw.lines().skip(2) {
        let Some((iface, stats)) = line.split_once(':') else {
            continue;
        };

        if iface.trim() == "lo" {
            continue;
        }

        let numbers: Vec<u64> = stats
            .split_whitespace()
            .filter_map(|field| field.parse::<u64>().ok())
            .collect();

        if numbers.len() >= 9 {
            rx_total = rx_total.saturating_add(numbers[0]);
            tx_total = tx_total.saturating_add(numbers[8]);
        }
    }

    CapabilityValue::available(NetworkMetrics {
        rx_bytes: rx_total,
        tx_bytes: tx_total,
    })
}

fn read_temperature() -> CapabilityValue<f32> {
    let Ok(entries) = fs::read_dir("/sys/class/thermal") else {
        return CapabilityValue::unsupported("thermal zone interface unavailable");
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .starts_with("thermal_zone")
        {
            continue;
        }

        let temp_path = path.join("temp");
        let Ok(raw) = fs::read_to_string(temp_path) else {
            continue;
        };

        let Ok(millidegrees) = raw.trim().parse::<f32>() else {
            continue;
        };

        return CapabilityValue::available(millidegrees / 1000.0);
    }

    CapabilityValue::unsupported("no readable thermal zones")
}
