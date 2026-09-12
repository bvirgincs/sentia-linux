use crate::errors::{ContractError, ContractErrorCode};
use crate::router::PROTOCOL_VERSION_V1;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolArea {
    Commands,
    Packages,
    Services,
    Logs,
    Resources,
    NetworkProcesses,
    FilesSystem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivilegeClass {
    User,
    ElevatedBroker,
    RootBroker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyClass {
    Public,
    Session,
    HostSensitive,
    Restricted,
    Secret,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancellationPolicy {
    NotCancellable,
    BestEffort,
    MustCancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolName {
    CommandExists,
    CommandLookup,
    CommandHelp,
    CommandHistory,
    AptSearch,
    AptPackageInfo,
    AptPackagePolicy,
    AptSimulateInstall,
    AptInstall,
    AptRemove,
    AptUpdate,
    AptUpgrade,
    PackageOwnsFile,
    ServiceList,
    ServiceStatus,
    ServiceStart,
    ServiceStop,
    ServiceRestart,
    ServiceEnable,
    JournalRecent,
    JournalErrors,
    KernelMessages,
    CpuStatus,
    MemoryStatus,
    SwapStatus,
    FilesystemStatus,
    DiskStatus,
    DiskHealth,
    TemperatureStatus,
    NetworkStatus,
    NetworkInterfaces,
    ListeningPorts,
    ProcessList,
    ProcessDetails,
    ProcessKill,
    FileExists,
    FileType,
    FilePermissions,
    DirectoryContents,
    DiskUsage,
    OsInformation,
    HardwareInformation,
}

impl ToolName {
    pub const ALL: [ToolName; 42] = [
        ToolName::CommandExists,
        ToolName::CommandLookup,
        ToolName::CommandHelp,
        ToolName::CommandHistory,
        ToolName::AptSearch,
        ToolName::AptPackageInfo,
        ToolName::AptPackagePolicy,
        ToolName::AptSimulateInstall,
        ToolName::AptInstall,
        ToolName::AptRemove,
        ToolName::AptUpdate,
        ToolName::AptUpgrade,
        ToolName::PackageOwnsFile,
        ToolName::ServiceList,
        ToolName::ServiceStatus,
        ToolName::ServiceStart,
        ToolName::ServiceStop,
        ToolName::ServiceRestart,
        ToolName::ServiceEnable,
        ToolName::JournalRecent,
        ToolName::JournalErrors,
        ToolName::KernelMessages,
        ToolName::CpuStatus,
        ToolName::MemoryStatus,
        ToolName::SwapStatus,
        ToolName::FilesystemStatus,
        ToolName::DiskStatus,
        ToolName::DiskHealth,
        ToolName::TemperatureStatus,
        ToolName::NetworkStatus,
        ToolName::NetworkInterfaces,
        ToolName::ListeningPorts,
        ToolName::ProcessList,
        ToolName::ProcessDetails,
        ToolName::ProcessKill,
        ToolName::FileExists,
        ToolName::FileType,
        ToolName::FilePermissions,
        ToolName::DirectoryContents,
        ToolName::DiskUsage,
        ToolName::OsInformation,
        ToolName::HardwareInformation,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ToolName::CommandExists => "command_exists",
            ToolName::CommandLookup => "command_lookup",
            ToolName::CommandHelp => "command_help",
            ToolName::CommandHistory => "command_history",
            ToolName::AptSearch => "apt_search",
            ToolName::AptPackageInfo => "apt_package_info",
            ToolName::AptPackagePolicy => "apt_package_policy",
            ToolName::AptSimulateInstall => "apt_simulate_install",
            ToolName::AptInstall => "apt_install",
            ToolName::AptRemove => "apt_remove",
            ToolName::AptUpdate => "apt_update",
            ToolName::AptUpgrade => "apt_upgrade",
            ToolName::PackageOwnsFile => "package_owns_file",
            ToolName::ServiceList => "service_list",
            ToolName::ServiceStatus => "service_status",
            ToolName::ServiceStart => "service_start",
            ToolName::ServiceStop => "service_stop",
            ToolName::ServiceRestart => "service_restart",
            ToolName::ServiceEnable => "service_enable",
            ToolName::JournalRecent => "journal_recent",
            ToolName::JournalErrors => "journal_errors",
            ToolName::KernelMessages => "kernel_messages",
            ToolName::CpuStatus => "cpu_status",
            ToolName::MemoryStatus => "memory_status",
            ToolName::SwapStatus => "swap_status",
            ToolName::FilesystemStatus => "filesystem_status",
            ToolName::DiskStatus => "disk_status",
            ToolName::DiskHealth => "disk_health",
            ToolName::TemperatureStatus => "temperature_status",
            ToolName::NetworkStatus => "network_status",
            ToolName::NetworkInterfaces => "network_interfaces",
            ToolName::ListeningPorts => "listening_ports",
            ToolName::ProcessList => "process_list",
            ToolName::ProcessDetails => "process_details",
            ToolName::ProcessKill => "process_kill",
            ToolName::FileExists => "file_exists",
            ToolName::FileType => "file_type",
            ToolName::FilePermissions => "file_permissions",
            ToolName::DirectoryContents => "directory_contents",
            ToolName::DiskUsage => "disk_usage",
            ToolName::OsInformation => "os_information",
            ToolName::HardwareInformation => "hardware_information",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolDefinition {
    pub name: ToolName,
    pub area: ToolArea,
    pub privilege_class: PrivilegeClass,
    pub privacy_class: PrivacyClass,
    pub input_schema_ref: String,
    pub output_schema_ref: String,
    pub provenance_required: bool,
    pub timeout_ms: u32,
    pub cancellation_policy: CancellationPolicy,
    pub max_input_bytes: u32,
    pub max_output_bytes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolRegistry {
    pub version: String,
    pub tools: Vec<ToolDefinition>,
}

impl ToolRegistry {
    pub fn required_v1() -> Self {
        let mut tools = Vec::with_capacity(ToolName::ALL.len());
        for name in ToolName::ALL {
            tools.push(required_definition(name));
        }

        Self {
            version: PROTOCOL_VERSION_V1.to_owned(),
            tools,
        }
    }

    pub fn validate_complete_required_set(&self) -> Result<(), ContractError> {
        if self.version != PROTOCOL_VERSION_V1 {
            return Err(ContractError::new(
                ContractErrorCode::UnsupportedVersion,
                "tool registry version is not sentia.v1",
                false,
            ));
        }

        let mut seen = BTreeSet::new();
        for tool in &self.tools {
            if !seen.insert(tool.name) {
                return Err(ContractError::new(
                    ContractErrorCode::ValidationFailed,
                    format!("duplicate tool registration: {}", tool.name.as_str()),
                    false,
                ));
            }
            if tool.max_input_bytes == 0 || tool.max_output_bytes == 0 {
                return Err(ContractError::new(
                    ContractErrorCode::ValidationFailed,
                    format!("tool {} has invalid payload bounds", tool.name.as_str()),
                    false,
                ));
            }
        }

        for required in ToolName::ALL {
            if !seen.contains(&required) {
                return Err(ContractError::new(
                    ContractErrorCode::ValidationFailed,
                    format!("missing required tool {}", required.as_str()),
                    false,
                ));
            }
        }

        Ok(())
    }
}

fn required_definition(name: ToolName) -> ToolDefinition {
    let (area, privilege_class, privacy_class, timeout_ms, cancellation_policy) = match name {
        ToolName::AptInstall | ToolName::AptRemove | ToolName::AptUpdate | ToolName::AptUpgrade => (
            ToolArea::Packages,
            PrivilegeClass::RootBroker,
            PrivacyClass::Restricted,
            120_000,
            CancellationPolicy::BestEffort,
        ),
        ToolName::ServiceStart
        | ToolName::ServiceStop
        | ToolName::ServiceRestart
        | ToolName::ServiceEnable => (
            ToolArea::Services,
            PrivilegeClass::RootBroker,
            PrivacyClass::Restricted,
            30_000,
            CancellationPolicy::BestEffort,
        ),
        ToolName::ProcessKill => (
            ToolArea::NetworkProcesses,
            PrivilegeClass::ElevatedBroker,
            PrivacyClass::HostSensitive,
            10_000,
            CancellationPolicy::BestEffort,
        ),
        ToolName::DiskHealth
        | ToolName::TemperatureStatus
        | ToolName::JournalRecent
        | ToolName::JournalErrors
        | ToolName::KernelMessages
        | ToolName::ProcessDetails => (
            inferred_area(name),
            PrivilegeClass::ElevatedBroker,
            PrivacyClass::HostSensitive,
            10_000,
            CancellationPolicy::MustCancel,
        ),
        _ => (
            inferred_area(name),
            PrivilegeClass::User,
            inferred_privacy(name),
            5_000,
            CancellationPolicy::MustCancel,
        ),
    };

    ToolDefinition {
        name,
        area,
        privilege_class,
        privacy_class,
        input_schema_ref: "schemas/tools/tool-invocation-v1.schema.json#/$defs/tool_input"
            .to_owned(),
        output_schema_ref: "schemas/tools/tool-invocation-v1.schema.json#/$defs/tool_output"
            .to_owned(),
        provenance_required: true,
        timeout_ms,
        cancellation_policy,
        max_input_bytes: 65_536,
        max_output_bytes: 262_144,
    }
}

fn inferred_area(name: ToolName) -> ToolArea {
    match name {
        ToolName::CommandExists
        | ToolName::CommandLookup
        | ToolName::CommandHelp
        | ToolName::CommandHistory => ToolArea::Commands,
        ToolName::AptSearch
        | ToolName::AptPackageInfo
        | ToolName::AptPackagePolicy
        | ToolName::AptSimulateInstall
        | ToolName::AptInstall
        | ToolName::AptRemove
        | ToolName::AptUpdate
        | ToolName::AptUpgrade
        | ToolName::PackageOwnsFile => ToolArea::Packages,
        ToolName::ServiceList
        | ToolName::ServiceStatus
        | ToolName::ServiceStart
        | ToolName::ServiceStop
        | ToolName::ServiceRestart
        | ToolName::ServiceEnable => ToolArea::Services,
        ToolName::JournalRecent | ToolName::JournalErrors | ToolName::KernelMessages => ToolArea::Logs,
        ToolName::CpuStatus
        | ToolName::MemoryStatus
        | ToolName::SwapStatus
        | ToolName::FilesystemStatus
        | ToolName::DiskStatus
        | ToolName::DiskHealth
        | ToolName::TemperatureStatus => ToolArea::Resources,
        ToolName::NetworkStatus
        | ToolName::NetworkInterfaces
        | ToolName::ListeningPorts
        | ToolName::ProcessList
        | ToolName::ProcessDetails
        | ToolName::ProcessKill => ToolArea::NetworkProcesses,
        ToolName::FileExists
        | ToolName::FileType
        | ToolName::FilePermissions
        | ToolName::DirectoryContents
        | ToolName::DiskUsage
        | ToolName::OsInformation
        | ToolName::HardwareInformation => ToolArea::FilesSystem,
    }
}

fn inferred_privacy(name: ToolName) -> PrivacyClass {
    match name {
        ToolName::CommandHistory => PrivacyClass::Session,
        ToolName::JournalRecent
        | ToolName::JournalErrors
        | ToolName::KernelMessages
        | ToolName::NetworkStatus
        | ToolName::NetworkInterfaces
        | ToolName::ListeningPorts
        | ToolName::ProcessList
        | ToolName::ProcessDetails
        | ToolName::ProcessKill
        | ToolName::FilePermissions
        | ToolName::DirectoryContents
        | ToolName::DiskUsage
        | ToolName::HardwareInformation
        | ToolName::DiskHealth
        | ToolName::TemperatureStatus => PrivacyClass::HostSensitive,
        ToolName::AptInstall
        | ToolName::AptRemove
        | ToolName::AptUpdate
        | ToolName::AptUpgrade
        | ToolName::ServiceStart
        | ToolName::ServiceStop
        | ToolName::ServiceRestart
        | ToolName::ServiceEnable => PrivacyClass::Restricted,
        _ => PrivacyClass::Public,
    }
}
