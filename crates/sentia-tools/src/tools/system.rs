use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::parsers::{parse_os_release, parse_proc_cpuinfo, parse_proc_meminfo};
use crate::result::{PrivacyClass, Provenance, ToolResult};
use crate::validation::read_text_file_bounded;
use crate::ToolError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OsInformationInput;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HardwareInformationInput;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlockDeviceSummary {
    pub name: String,
    pub size_bytes: Option<u64>,
    pub removable: Option<bool>,
    pub rotational: Option<bool>,
}

pub fn os_information(_input: OsInformationInput) -> Result<ToolResult, ToolError> {
    let os_release_text = read_text_file_bounded(Path::new("/etc/os-release"), 128 * 1024)?;
    let os_release = parse_os_release(&os_release_text);

    let kernel = BTreeMap::from([
        (
            "ostype".to_string(),
            read_optional_kernel_field("/proc/sys/kernel/ostype"),
        ),
        (
            "osrelease".to_string(),
            read_optional_kernel_field("/proc/sys/kernel/osrelease"),
        ),
        (
            "version".to_string(),
            read_optional_kernel_field("/proc/sys/kernel/version"),
        ),
    ]);

    Ok(ToolResult::new(
        "os_information",
        PrivacyClass::SystemMetadata,
        Duration::from_secs(1),
        vec![
            Provenance {
                source: "filesystem".to_string(),
                detail: "/etc/os-release".to_string(),
            },
            Provenance {
                source: "procfs".to_string(),
                detail: "/proc/sys/kernel/{ostype,osrelease,version}".to_string(),
            },
        ],
        false,
        json!({
            "os_release": os_release,
            "kernel": kernel,
            "arch": std::env::consts::ARCH,
            "family": std::env::consts::FAMILY,
        }),
    ))
}

pub fn hardware_information(_input: HardwareInformationInput) -> Result<ToolResult, ToolError> {
    let cpuinfo = read_text_file_bounded(Path::new("/proc/cpuinfo"), 512 * 1024)?;
    let meminfo = read_text_file_bounded(Path::new("/proc/meminfo"), 512 * 1024)?;

    let cpu = parse_proc_cpuinfo(&cpuinfo);
    let mem = parse_proc_meminfo(&meminfo);

    let dmi = BTreeMap::from([
        (
            "sys_vendor".to_string(),
            read_optional_sysfs_field("/sys/class/dmi/id/sys_vendor"),
        ),
        (
            "product_name".to_string(),
            read_optional_sysfs_field("/sys/class/dmi/id/product_name"),
        ),
        (
            "product_version".to_string(),
            read_optional_sysfs_field("/sys/class/dmi/id/product_version"),
        ),
        (
            "bios_vendor".to_string(),
            read_optional_sysfs_field("/sys/class/dmi/id/bios_vendor"),
        ),
        (
            "bios_version".to_string(),
            read_optional_sysfs_field("/sys/class/dmi/id/bios_version"),
        ),
    ]);

    let block_devices = collect_block_devices()?;

    Ok(ToolResult::new(
        "hardware_information",
        PrivacyClass::SystemMetadata,
        Duration::from_secs(1),
        vec![
            Provenance {
                source: "procfs".to_string(),
                detail: "/proc/cpuinfo, /proc/meminfo".to_string(),
            },
            Provenance {
                source: "sysfs".to_string(),
                detail: "/sys/class/dmi/id/*, /sys/block/*".to_string(),
            },
        ],
        false,
        json!({
            "cpu": cpu,
            "memory": mem,
            "dmi": dmi,
            "block_devices": block_devices,
        }),
    ))
}

fn collect_block_devices() -> Result<Vec<BlockDeviceSummary>, ToolError> {
    let mut devices = Vec::<BlockDeviceSummary>::new();

    let entries = fs::read_dir("/sys/block")
        .map_err(|err| ToolError::io("reading /sys/block", err.to_string()))?;

    for entry in entries {
        let entry = entry.map_err(|err| ToolError::io("iterating /sys/block", err.to_string()))?;
        let name = entry.file_name().to_string_lossy().to_string();
        let base = entry.path();

        let sectors = read_optional_sysfs_field(base.join("size")).and_then(|value| value.parse::<u64>().ok());
        let size_bytes = sectors.map(|sector_count| sector_count.saturating_mul(512));

        let removable = read_optional_sysfs_field(base.join("removable"))
            .and_then(|value| match value.as_str() {
                "0" => Some(false),
                "1" => Some(true),
                _ => None,
            });

        let rotational = read_optional_sysfs_field(base.join("queue/rotational"))
            .and_then(|value| match value.as_str() {
                "0" => Some(false),
                "1" => Some(true),
                _ => None,
            });

        devices.push(BlockDeviceSummary {
            name,
            size_bytes,
            removable,
            rotational,
        });
    }

    devices.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(devices)
}

fn read_optional_kernel_field(path: &str) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn read_optional_sysfs_field(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
