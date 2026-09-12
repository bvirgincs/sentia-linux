use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityState {
    Available,
    MissingTool,
    PermissionDenied,
    Unavailable,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Capability {
    pub state: CapabilityState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Capability {
    pub fn available() -> Self {
        Self {
            state: CapabilityState::Available,
            detail: None,
        }
    }

    pub fn missing_tool(tool: impl Into<String>) -> Self {
        Self {
            state: CapabilityState::MissingTool,
            detail: Some(tool.into()),
        }
    }

    pub fn permission_denied(detail: impl Into<String>) -> Self {
        Self {
            state: CapabilityState::PermissionDenied,
            detail: Some(detail.into()),
        }
    }

    pub fn unavailable(detail: impl Into<String>) -> Self {
        Self {
            state: CapabilityState::Unavailable,
            detail: Some(detail.into()),
        }
    }

    pub fn error(detail: impl Into<String>) -> Self {
        Self {
            state: CapabilityState::Error,
            detail: Some(detail.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolStatus {
    pub smartctl: Capability,
    pub nvme: Capability,
    pub systemctl: Capability,
    pub journalctl: Capability,
    pub thermal_sysfs: Capability,
}

impl Default for ToolStatus {
    fn default() -> Self {
        Self {
            smartctl: Capability::unavailable("not-probed"),
            nvme: Capability::unavailable("not-probed"),
            systemctl: Capability::unavailable("not-probed"),
            journalctl: Capability::unavailable("not-probed"),
            thermal_sysfs: Capability::unavailable("not-probed"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CpuStatus {
    pub utilization_pct: f64,
    pub load_1: f64,
    pub load_5: f64,
    pub load_15: f64,
    pub logical_cores: usize,
    pub counter_reset: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryPressure {
    pub some_avg10: f64,
    pub some_avg60: f64,
    pub some_avg300: f64,
    pub full_avg10: f64,
    pub full_avg60: f64,
    pub full_avg300: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryStatus {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
    pub used_pct: f64,
    pub pressure: Option<MemoryPressure>,
    pub pressure_capability: Capability,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SwapStatus {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub free_bytes: u64,
    pub used_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FilesystemSample {
    pub mount_point: String,
    pub fs_type: String,
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
    pub used_pct: f64,
    pub full: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FilesystemStatus {
    pub filesystems: Vec<FilesystemSample>,
    pub full_filesystems: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiskIoSample {
    pub device: String,
    pub read_bps: f64,
    pub write_bps: f64,
    pub utilization_pct: f64,
    pub counter_reset: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiskStatus {
    pub devices: Vec<DiskIoSample>,
    pub total_read_bps: f64,
    pub total_write_bps: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiskHealthState {
    Healthy,
    Degraded,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiskHealthDevice {
    pub device: String,
    pub backend: String,
    pub health: DiskHealthState,
    pub capability: Capability,
    pub temperature_c: Option<f64>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiskHealthStatus {
    pub overall: DiskHealthState,
    pub devices: Vec<DiskHealthDevice>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TemperatureSample {
    pub name: String,
    pub temperature_c: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TemperatureStatus {
    pub sensors: Vec<TemperatureSample>,
    pub max_c: Option<f64>,
    pub capability: Capability,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NetworkSample {
    pub interface: String,
    pub rx_bps: f64,
    pub tx_bps: f64,
    pub utilization_pct: Option<f64>,
    pub speed_mbps: Option<u64>,
    pub counter_reset: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NetworkStatus {
    pub interfaces: Vec<NetworkSample>,
    pub total_rx_bps: f64,
    pub total_tx_bps: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProcessSample {
    pub pid: u32,
    pub name: String,
    pub cpu_pct: f64,
    pub memory_bytes: u64,
    pub memory_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProcessStatus {
    pub high_resource: Vec<ProcessSample>,
    pub churned_processes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FailedUnit {
    pub unit: String,
    pub active_state: String,
    pub sub_state: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SystemdStatus {
    pub failed_units: Vec<FailedUnit>,
    pub capability: Capability,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KernelError {
    pub timestamp: Option<String>,
    pub priority: Option<u8>,
    pub identifier: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KernelStatus {
    pub recent_errors: Vec<KernelError>,
    pub truncated: bool,
    pub capability: Capability,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AlertLevel {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlertRecord {
    pub id: String,
    pub level: AlertLevel,
    pub active: bool,
    pub message: String,
    pub value: f64,
    pub first_seen_unix_ms: u64,
    pub last_seen_unix_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HealthSnapshot {
    pub schema_version: u32,
    pub collected_at_unix_ms: u64,
    pub elapsed_ms: u64,
    pub cpu: CpuStatus,
    pub memory: MemoryStatus,
    pub swap: SwapStatus,
    pub filesystem: FilesystemStatus,
    pub disk: DiskStatus,
    pub diskhealth: DiskHealthStatus,
    pub temperature: TemperatureStatus,
    pub network: NetworkStatus,
    pub processes: ProcessStatus,
    pub systemd: SystemdStatus,
    pub kernel: KernelStatus,
    pub tools: ToolStatus,
    pub alerts: Vec<AlertRecord>,
    pub alert_log: Vec<AlertRecord>,
}

impl HealthSnapshot {
    pub fn empty(now_unix_ms: u64) -> Self {
        Self {
            schema_version: 1,
            collected_at_unix_ms: now_unix_ms,
            elapsed_ms: 0,
            cpu: CpuStatus {
                utilization_pct: 0.0,
                load_1: 0.0,
                load_5: 0.0,
                load_15: 0.0,
                logical_cores: 0,
                counter_reset: false,
            },
            memory: MemoryStatus {
                total_bytes: 0,
                used_bytes: 0,
                available_bytes: 0,
                used_pct: 0.0,
                pressure: None,
                pressure_capability: Capability::unavailable("not-sampled"),
            },
            swap: SwapStatus {
                total_bytes: 0,
                used_bytes: 0,
                free_bytes: 0,
                used_pct: 0.0,
            },
            filesystem: FilesystemStatus {
                filesystems: Vec::new(),
                full_filesystems: 0,
            },
            disk: DiskStatus {
                devices: Vec::new(),
                total_read_bps: 0.0,
                total_write_bps: 0.0,
            },
            diskhealth: DiskHealthStatus {
                overall: DiskHealthState::Unknown,
                devices: Vec::new(),
            },
            temperature: TemperatureStatus {
                sensors: Vec::new(),
                max_c: None,
                capability: Capability::unavailable("not-sampled"),
            },
            network: NetworkStatus {
                interfaces: Vec::new(),
                total_rx_bps: 0.0,
                total_tx_bps: 0.0,
            },
            processes: ProcessStatus {
                high_resource: Vec::new(),
                churned_processes: 0,
            },
            systemd: SystemdStatus {
                failed_units: Vec::new(),
                capability: Capability::unavailable("not-sampled"),
            },
            kernel: KernelStatus {
                recent_errors: Vec::new(),
                truncated: false,
                capability: Capability::unavailable("not-sampled"),
            },
            tools: ToolStatus::default(),
            alerts: Vec::new(),
            alert_log: Vec::new(),
        }
    }
}
