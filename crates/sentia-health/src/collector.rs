use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use nix::sys::statvfs::statvfs;
use serde_json::Value;

use crate::procfs::{
    cpu_utilization_pct, parse_cpu_total, parse_diskstats, parse_loadavg, parse_meminfo,
    parse_memory_pressure, parse_net_dev, parse_process_stat, sanitize_log_message, CpuTimes,
    DiskCounters, NetCounters,
};
use crate::schema::{
    AlertRecord, Capability, CapabilityState, CpuStatus, DiskHealthDevice, DiskHealthState,
    DiskHealthStatus, DiskIoSample, DiskStatus, FailedUnit, FilesystemSample, FilesystemStatus,
    HealthSnapshot, KernelError, KernelStatus, MemoryPressure, MemoryStatus, NetworkSample,
    NetworkStatus, ProcessSample, ProcessStatus, SwapStatus, SystemdStatus, TemperatureSample,
    TemperatureStatus, ToolStatus,
};

const SMARTCTL_PATHS: &[&str] = &["/usr/sbin/smartctl", "/usr/bin/smartctl"];
const NVME_PATHS: &[&str] = &["/usr/sbin/nvme", "/usr/bin/nvme"];
const SYSTEMCTL_PATHS: &[&str] = &["/usr/bin/systemctl", "/bin/systemctl"];
const JOURNALCTL_PATHS: &[&str] = &["/usr/bin/journalctl", "/bin/journalctl"];

#[derive(Debug, Clone)]
pub struct CollectorConfig {
    pub max_filesystems: usize,
    pub max_processes: usize,
    pub max_kernel_errors: usize,
    pub max_failed_units: usize,
    pub max_network_interfaces: usize,
    pub max_disks: usize,
    pub max_disk_health_devices: usize,
}

impl Default for CollectorConfig {
    fn default() -> Self {
        Self {
            max_filesystems: 32,
            max_processes: 10,
            max_kernel_errors: 16,
            max_failed_units: 32,
            max_network_interfaces: 16,
            max_disks: 16,
            max_disk_health_devices: 8,
        }
    }
}

#[derive(Debug)]
pub struct HealthCollector {
    config: CollectorConfig,
    prev_sample_at: Option<Instant>,
    prev_cpu: Option<CpuTimes>,
    prev_cpu_total: Option<u64>,
    prev_net: HashMap<String, NetCounters>,
    prev_disk: HashMap<String, DiskCounters>,
    prev_process_cpu: HashMap<u32, u64>,
    page_size: u64,
}

impl HealthCollector {
    pub fn new(config: CollectorConfig) -> Self {
        // SAFETY: sysconf with _SC_PAGESIZE has no side effects and does not dereference pointers.
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        let page_size = if page_size <= 0 { 4096 } else { page_size as u64 };

        Self {
            config,
            prev_sample_at: None,
            prev_cpu: None,
            prev_cpu_total: None,
            prev_net: HashMap::new(),
            prev_disk: HashMap::new(),
            prev_process_cpu: HashMap::new(),
            page_size,
        }
    }

    pub fn collect(&mut self) -> HealthSnapshot {
        let now_unix_ms = unix_now_ms();
        let elapsed_ms = self
            .prev_sample_at
            .map(|last| {
                let elapsed = last.elapsed().as_millis();
                if elapsed == 0 {
                    0
                } else {
                    elapsed.min(u128::from(u64::MAX)) as u64
                }
            })
            .unwrap_or(0);
        self.prev_sample_at = Some(Instant::now());

        let mut snapshot = HealthSnapshot::empty(now_unix_ms);
        snapshot.elapsed_ms = elapsed_ms;

        let mut tools = ToolStatus::default();

        let (cpu, cpu_total_delta) = self.collect_cpu();
        snapshot.cpu = cpu;

        let (memory, swap) = self.collect_memory();
        snapshot.memory = memory;
        snapshot.swap = swap;

        snapshot.filesystem = self.collect_filesystems();

        snapshot.disk = self.collect_disk(elapsed_ms);
        snapshot.network = self.collect_network(elapsed_ms);
        snapshot.processes = self.collect_processes(cpu_total_delta, snapshot.cpu.logical_cores, snapshot.memory.total_bytes);

        let (temperature, thermal_capability) = self.collect_temperature_from_root(Path::new("/sys/class/thermal"));
        snapshot.temperature = temperature;
        tools.thermal_sysfs = thermal_capability;

        let (systemd, systemctl_capability) = self.collect_failed_units();
        snapshot.systemd = systemd;
        tools.systemctl = systemctl_capability;

        let (kernel, journalctl_capability) = self.collect_kernel_errors();
        snapshot.kernel = kernel;
        tools.journalctl = journalctl_capability;

        let (diskhealth, smartctl_cap, nvme_cap) = self.collect_disk_health();
        snapshot.diskhealth = diskhealth;
        tools.smartctl = smartctl_cap;
        tools.nvme = nvme_cap;

        snapshot.tools = tools;
        snapshot
    }

    fn collect_cpu(&mut self) -> (CpuStatus, Option<u64>) {
        let stat = fs::read_to_string("/proc/stat").ok();
        let loadavg = fs::read_to_string("/proc/loadavg").ok();

        let cores = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1);

        let mut cpu = CpuStatus {
            utilization_pct: 0.0,
            load_1: 0.0,
            load_5: 0.0,
            load_15: 0.0,
            logical_cores: cores,
            counter_reset: false,
        };

        let mut total_delta = None;

        if let Some(ref raw) = stat {
            if let Some(current) = parse_cpu_total(raw) {
                if let Some(previous) = self.prev_cpu {
                    match cpu_utilization_pct(current, previous) {
                        Some(value) => {
                            cpu.utilization_pct = value;
                            if let Some(delta) = current.delta(previous) {
                                total_delta = Some(delta.total);
                                self.prev_cpu_total = Some(delta.total);
                            }
                        }
                        None => {
                            cpu.counter_reset = true;
                            cpu.utilization_pct = 0.0;
                        }
                    }
                }
                self.prev_cpu = Some(current);
            }
        }

        if let Some(ref load_content) = loadavg {
            if let Some((load_1, load_5, load_15)) = parse_loadavg(load_content) {
                cpu.load_1 = load_1;
                cpu.load_5 = load_5;
                cpu.load_15 = load_15;
            }
        }

        (cpu, total_delta)
    }

    fn collect_memory(&self) -> (MemoryStatus, SwapStatus) {
        let meminfo_content = fs::read_to_string("/proc/meminfo").ok();
        let psi_content = fs::read_to_string("/proc/pressure/memory").ok();

        let mut memory = MemoryStatus {
            total_bytes: 0,
            used_bytes: 0,
            available_bytes: 0,
            used_pct: 0.0,
            pressure: None,
            pressure_capability: Capability::unavailable("/proc/pressure/memory unavailable"),
        };

        let mut swap = SwapStatus {
            total_bytes: 0,
            used_bytes: 0,
            free_bytes: 0,
            used_pct: 0.0,
        };

        if let Some(content) = meminfo_content {
            if let Some(meminfo) = parse_meminfo(&content) {
                let used_bytes = meminfo.total_bytes.saturating_sub(meminfo.available_bytes);
                let used_pct = pct(used_bytes, meminfo.total_bytes);

                memory.total_bytes = meminfo.total_bytes;
                memory.available_bytes = meminfo.available_bytes;
                memory.used_bytes = used_bytes;
                memory.used_pct = used_pct;

                swap.total_bytes = meminfo.swap_total_bytes;
                swap.free_bytes = meminfo.swap_free_bytes;
                swap.used_bytes = meminfo
                    .swap_total_bytes
                    .saturating_sub(meminfo.swap_free_bytes);
                swap.used_pct = pct(swap.used_bytes, swap.total_bytes);
            }
        }

        if let Some(content) = psi_content {
            if let Some(parsed) = parse_memory_pressure(&content) {
                memory.pressure = Some(parsed);
                memory.pressure_capability = Capability::available();
            } else {
                memory.pressure_capability = Capability::error("failed to parse /proc/pressure/memory");
            }
        }

        (memory, swap)
    }

    fn collect_filesystems(&self) -> FilesystemStatus {
        let mounts = fs::read_to_string("/proc/self/mounts").unwrap_or_default();
        let mut seen = HashSet::new();
        let mut samples = Vec::new();

        for line in mounts.lines() {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 3 {
                continue;
            }

            let mount_point = unescape_mount(fields[1]);
            let fs_type = fields[2].to_string();

            if should_skip_fs(&fs_type) || !seen.insert(mount_point.clone()) {
                continue;
            }

            let Ok(stats) = statvfs(Path::new(&mount_point)) else {
                continue;
            };

            let total_bytes = stats.blocks().saturating_mul(stats.fragment_size());
            let available_bytes = stats.blocks_available().saturating_mul(stats.fragment_size());
            let free_bytes = stats.blocks_free().saturating_mul(stats.fragment_size());
            let used_bytes = total_bytes.saturating_sub(free_bytes);
            let used_pct = pct(used_bytes, total_bytes);
            let full = available_bytes == 0 || used_pct >= 99.0;

            samples.push(FilesystemSample {
                mount_point,
                fs_type,
                total_bytes,
                used_bytes,
                available_bytes,
                used_pct,
                full,
            });
        }

        samples.sort_by(|a, b| {
            b.used_pct
                .partial_cmp(&a.used_pct)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        if samples.len() > self.config.max_filesystems {
            samples.truncate(self.config.max_filesystems);
        }

        let full_filesystems = samples.iter().filter(|entry| entry.full).count();
        FilesystemStatus {
            filesystems: samples,
            full_filesystems,
        }
    }

    fn collect_disk(&mut self, elapsed_ms: u64) -> DiskStatus {
        let content = fs::read_to_string("/proc/diskstats").unwrap_or_default();
        let current = parse_diskstats(&content);

        let elapsed_sec = (elapsed_ms as f64) / 1000.0;

        let mut devices = Vec::new();
        let mut total_read = 0.0;
        let mut total_write = 0.0;

        for (device, counters) in &current {
            let mut counter_reset = false;
            let mut read_delta = 0_u64;
            let mut write_delta = 0_u64;
            let mut io_time_delta = 0_u64;

            if let Some(previous) = self.prev_disk.get(device) {
                let (d, reset) = counter_delta(counters.read_bytes, previous.read_bytes);
                read_delta = d;
                counter_reset |= reset;

                let (d, reset) = counter_delta(counters.write_bytes, previous.write_bytes);
                write_delta = d;
                counter_reset |= reset;

                let (d, reset) = counter_delta(counters.io_time_ms, previous.io_time_ms);
                io_time_delta = d;
                counter_reset |= reset;
            }

            let read_bps = if elapsed_sec > 0.0 {
                (read_delta as f64) / elapsed_sec
            } else {
                0.0
            };
            let write_bps = if elapsed_sec > 0.0 {
                (write_delta as f64) / elapsed_sec
            } else {
                0.0
            };
            let utilization_pct = if elapsed_ms > 0 {
                ((io_time_delta as f64) / (elapsed_ms as f64) * 100.0).clamp(0.0, 100.0)
            } else {
                0.0
            };

            total_read += read_bps;
            total_write += write_bps;

            devices.push(DiskIoSample {
                device: device.clone(),
                read_bps,
                write_bps,
                utilization_pct,
                counter_reset,
            });
        }

        devices.sort_by(|a, b| {
            let a_total = a.read_bps + a.write_bps;
            let b_total = b.read_bps + b.write_bps;
            b_total
                .partial_cmp(&a_total)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        if devices.len() > self.config.max_disks {
            devices.truncate(self.config.max_disks);
        }

        self.prev_disk = current;

        DiskStatus {
            devices,
            total_read_bps: total_read,
            total_write_bps: total_write,
        }
    }

    fn collect_network(&mut self, elapsed_ms: u64) -> NetworkStatus {
        let content = fs::read_to_string("/proc/net/dev").unwrap_or_default();
        let current = parse_net_dev(&content);
        let elapsed_sec = (elapsed_ms as f64) / 1000.0;

        let mut interfaces = Vec::new();
        let mut total_rx_bps = 0.0;
        let mut total_tx_bps = 0.0;

        for (iface, counters) in &current {
            let mut reset = false;
            let mut rx_delta = 0_u64;
            let mut tx_delta = 0_u64;

            if let Some(previous) = self.prev_net.get(iface) {
                let (d, was_reset) = counter_delta(counters.rx_bytes, previous.rx_bytes);
                rx_delta = d;
                reset |= was_reset;

                let (d, was_reset) = counter_delta(counters.tx_bytes, previous.tx_bytes);
                tx_delta = d;
                reset |= was_reset;
            }

            let rx_bps = if elapsed_sec > 0.0 {
                (rx_delta as f64) / elapsed_sec
            } else {
                0.0
            };
            let tx_bps = if elapsed_sec > 0.0 {
                (tx_delta as f64) / elapsed_sec
            } else {
                0.0
            };

            let speed_mbps = read_link_speed_mbps(iface);
            let utilization_pct = speed_mbps.and_then(|speed| {
                if speed == 0 {
                    return None;
                }
                let bps = (rx_bps + tx_bps) * 8.0;
                Some((bps / ((speed as f64) * 1_000_000.0) * 100.0).clamp(0.0, 100.0))
            });

            total_rx_bps += rx_bps;
            total_tx_bps += tx_bps;

            interfaces.push(NetworkSample {
                interface: iface.clone(),
                rx_bps,
                tx_bps,
                utilization_pct,
                speed_mbps,
                counter_reset: reset,
            });
        }

        interfaces.sort_by(|a, b| {
            let a_total = a.rx_bps + a.tx_bps;
            let b_total = b.rx_bps + b.tx_bps;
            b_total
                .partial_cmp(&a_total)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        if interfaces.len() > self.config.max_network_interfaces {
            interfaces.truncate(self.config.max_network_interfaces);
        }

        self.prev_net = current;

        NetworkStatus {
            interfaces,
            total_rx_bps,
            total_tx_bps,
        }
    }

    fn collect_processes(
        &mut self,
        cpu_total_delta: Option<u64>,
        cores: usize,
        total_memory_bytes: u64,
    ) -> ProcessStatus {
        let mut samples = Vec::new();
        let mut current_map = HashMap::new();

        let process_dirs = fs::read_dir("/proc");
        let Ok(process_dirs) = process_dirs else {
            return ProcessStatus {
                high_resource: Vec::new(),
                churned_processes: 0,
            };
        };

        for entry in process_dirs.flatten() {
            let name = entry.file_name();
            let pid_text = name.to_string_lossy();
            if !pid_text.chars().all(|ch| ch.is_ascii_digit()) {
                continue;
            }

            let stat_path = entry.path().join("stat");
            let Ok(stat_content) = fs::read_to_string(stat_path) else {
                continue;
            };
            let Some(stat) = parse_process_stat(&stat_content) else {
                continue;
            };

            current_map.insert(stat.pid, stat.cpu_jiffies);

            let cpu_pct = match (
                cpu_total_delta,
                self.prev_process_cpu.get(&stat.pid).copied(),
                cores,
            ) {
                (Some(total_delta), Some(prev), core_count) if total_delta > 0 && stat.cpu_jiffies >= prev => {
                    ((stat.cpu_jiffies - prev) as f64 / total_delta as f64 * 100.0 * core_count as f64)
                        .clamp(0.0, 100.0 * core_count as f64)
                }
                _ => 0.0,
            };

            let memory_bytes = if stat.rss_pages <= 0 {
                0
            } else {
                (stat.rss_pages as u64).saturating_mul(self.page_size)
            };
            let memory_pct = pct(memory_bytes, total_memory_bytes);

            samples.push(ProcessSample {
                pid: stat.pid,
                name: stat.name,
                cpu_pct,
                memory_bytes,
                memory_pct,
            });
        }

        let previous_pids: HashSet<u32> = self.prev_process_cpu.keys().copied().collect();
        let current_pids: HashSet<u32> = current_map.keys().copied().collect();
        let churned_processes = previous_pids
            .symmetric_difference(&current_pids)
            .count();

        samples.sort_by(|a, b| {
            let a_score = a.cpu_pct + a.memory_pct;
            let b_score = b.cpu_pct + b.memory_pct;
            b_score
                .partial_cmp(&a_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        if samples.len() > self.config.max_processes {
            samples.truncate(self.config.max_processes);
        }

        self.prev_process_cpu = current_map;

        ProcessStatus {
            high_resource: samples,
            churned_processes,
        }
    }

    pub fn collect_temperature_from_root(&self, root: &Path) -> (TemperatureStatus, Capability) {
        let mut sensors = Vec::new();

        let read_dir = fs::read_dir(root);
        let Ok(entries) = read_dir else {
            let capability = Capability::unavailable(format!("{} unavailable", root.display()));
            return (
                TemperatureStatus {
                    sensors,
                    max_c: None,
                    capability: capability.clone(),
                },
                capability,
            );
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = entry.file_name();
            let file_name = file_name.to_string_lossy();
            if !file_name.starts_with("thermal_zone") {
                continue;
            }

            let temp_path = path.join("temp");
            let type_path = path.join("type");

            let Ok(temp_raw) = fs::read_to_string(temp_path) else {
                continue;
            };

            let Ok(temp_value) = temp_raw.trim().parse::<f64>() else {
                continue;
            };

            let name = fs::read_to_string(type_path)
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| file_name.to_string());

            let temp_c = if temp_value > 200.0 {
                temp_value / 1000.0
            } else {
                temp_value
            };

            sensors.push(TemperatureSample {
                name,
                temperature_c: temp_c,
            });
        }

        sensors.sort_by(|a, b| {
            b.temperature_c
                .partial_cmp(&a.temperature_c)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let max_c = sensors
            .iter()
            .map(|sensor| sensor.temperature_c)
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let capability = if sensors.is_empty() {
            Capability::unavailable("no temperature sensors found")
        } else {
            Capability::available()
        };

        (
            TemperatureStatus {
                sensors,
                max_c,
                capability: capability.clone(),
            },
            capability,
        )
    }

    fn collect_failed_units(&self) -> (SystemdStatus, Capability) {
        let command_result = run_fixed_command(SYSTEMCTL_PATHS, &["--no-pager", "--no-legend", "--plain", "--failed"]);

        match command_result {
            Ok(output) => {
                if !output.success {
                    let joined = format!("{}\n{}", output.stdout, output.stderr);
                    if joined.contains("System has not been booted with systemd") {
                        let capability = Capability::unavailable("systemd not running");
                        return (
                            SystemdStatus {
                                failed_units: Vec::new(),
                                capability: capability.clone(),
                            },
                            capability,
                        );
                    }

                    if is_permission_denied(&joined) {
                        let capability = Capability::permission_denied("systemctl access denied");
                        return (
                            SystemdStatus {
                                failed_units: Vec::new(),
                                capability: capability.clone(),
                            },
                            capability,
                        );
                    }

                    let capability = Capability::error("systemctl execution failed");
                    return (
                        SystemdStatus {
                            failed_units: Vec::new(),
                            capability: capability.clone(),
                        },
                        capability,
                    );
                }

                let mut units = Vec::new();
                for line in output.stdout.lines() {
                    let fields: Vec<&str> = line.split_whitespace().collect();
                    if fields.len() < 4 {
                        continue;
                    }
                    let description = if fields.len() > 4 {
                        fields[4..].join(" ")
                    } else {
                        String::new()
                    };

                    units.push(FailedUnit {
                        unit: fields[0].to_string(),
                        active_state: fields[2].to_string(),
                        sub_state: fields[3].to_string(),
                        description,
                    });
                }

                if units.len() > self.config.max_failed_units {
                    units.truncate(self.config.max_failed_units);
                }

                let capability = Capability::available();
                (
                    SystemdStatus {
                        failed_units: units,
                        capability: capability.clone(),
                    },
                    capability,
                )
            }
            Err(capability) => {
                (
                    SystemdStatus {
                        failed_units: Vec::new(),
                        capability: capability.clone(),
                    },
                    capability,
                )
            }
        }
    }

    fn collect_kernel_errors(&self) -> (KernelStatus, Capability) {
        let max_entries_text = self.config.max_kernel_errors.to_string();
        let command_result = run_fixed_command(
            JOURNALCTL_PATHS,
            &[
                "-k",
                "-p",
                "err..alert",
                "-n",
                &max_entries_text,
                "--no-pager",
                "--output=json",
            ],
        );

        match command_result {
            Ok(output) => {
                let joined = format!("{}\n{}", output.stdout, output.stderr);
                if !output.success {
                    if is_permission_denied(&joined)
                        || joined.contains("No journal files were opened due to insufficient permissions")
                    {
                        let capability = Capability::permission_denied("journalctl requires journal read permission");
                        return (
                            KernelStatus {
                                recent_errors: Vec::new(),
                                truncated: false,
                                capability: capability.clone(),
                            },
                            capability,
                        );
                    }

                    if joined.contains("No journal files were found") {
                        let capability = Capability::unavailable("journal unavailable");
                        return (
                            KernelStatus {
                                recent_errors: Vec::new(),
                                truncated: false,
                                capability: capability.clone(),
                            },
                            capability,
                        );
                    }

                    let capability = Capability::error("journalctl execution failed");
                    return (
                        KernelStatus {
                            recent_errors: Vec::new(),
                            truncated: false,
                            capability: capability.clone(),
                        },
                        capability,
                    );
                }

                let mut errors = Vec::new();
                for line in output.stdout.lines() {
                    if line.trim().is_empty() {
                        continue;
                    }

                    let Ok(value) = serde_json::from_str::<Value>(line) else {
                        continue;
                    };

                    let message = value
                        .get("MESSAGE")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let message = sanitize_log_message(message, 180);

                    let timestamp = value
                        .get("__REALTIME_TIMESTAMP")
                        .and_then(Value::as_str)
                        .map(|text| text.to_string());
                    let priority = value
                        .get("PRIORITY")
                        .and_then(Value::as_str)
                        .and_then(|text| text.parse::<u8>().ok());
                    let identifier = value
                        .get("SYSLOG_IDENTIFIER")
                        .and_then(Value::as_str)
                        .map(|text| text.to_string());

                    errors.push(KernelError {
                        timestamp,
                        priority,
                        identifier,
                        message,
                    });
                }

                let truncated = errors.len() >= self.config.max_kernel_errors;
                if errors.len() > self.config.max_kernel_errors {
                    errors.truncate(self.config.max_kernel_errors);
                }

                let capability = Capability::available();
                (
                    KernelStatus {
                        recent_errors: errors,
                        truncated,
                        capability: capability.clone(),
                    },
                    capability,
                )
            }
            Err(capability) => (
                KernelStatus {
                    recent_errors: Vec::new(),
                    truncated: false,
                    capability: capability.clone(),
                },
                capability,
            ),
        }
    }

    fn collect_disk_health(&self) -> (DiskHealthStatus, Capability, Capability) {
        let devices = list_block_devices(self.config.max_disk_health_devices);

        let mut smartctl_capability = resolve_tool_presence(SMARTCTL_PATHS, "smartctl");
        let mut nvme_capability = resolve_tool_presence(NVME_PATHS, "nvme");

        let mut samples = Vec::new();

        for device in devices {
            if device.starts_with("nvme") {
                let (sample, capability) = self.probe_nvme_device(&device);
                if capability.state != CapabilityState::Available {
                    nvme_capability = capability.clone();
                }
                samples.push(sample);
                continue;
            }

            let (sample, capability) = self.probe_smartctl_device(&device);
            if capability.state != CapabilityState::Available {
                smartctl_capability = capability.clone();
            }
            samples.push(sample);
        }

        let overall = if samples.iter().any(|sample| sample.health == DiskHealthState::Degraded) {
            DiskHealthState::Degraded
        } else if samples.iter().any(|sample| sample.health == DiskHealthState::Healthy) {
            DiskHealthState::Healthy
        } else {
            DiskHealthState::Unknown
        };

        (
            DiskHealthStatus {
                overall,
                devices: samples,
            },
            smartctl_capability,
            nvme_capability,
        )
    }

    fn probe_smartctl_device(&self, device: &str) -> (DiskHealthDevice, Capability) {
        let dev_path = format!("/dev/{device}");
        let args = ["-H", "-n", "standby", dev_path.as_str()];

        match run_fixed_command(SMARTCTL_PATHS, &args) {
            Ok(output) => {
                let body = format!("{}\n{}", output.stdout, output.stderr);
                if is_permission_denied(&body) {
                    let capability = Capability::permission_denied(format!("smartctl denied for {device}"));
                    return (
                        DiskHealthDevice {
                            device: device.to_string(),
                            backend: "smartctl".to_string(),
                            health: DiskHealthState::Unknown,
                            capability: capability.clone(),
                            temperature_c: None,
                            note: Some("read-only probe blocked by permission".to_string()),
                        },
                        capability,
                    );
                }

                let lower = body.to_ascii_lowercase();
                let health = if lower.contains("test result: passed") || lower.contains("health status: ok") {
                    DiskHealthState::Healthy
                } else if lower.contains("test result: failed") || lower.contains("health status: critical") {
                    DiskHealthState::Degraded
                } else {
                    DiskHealthState::Unknown
                };

                let note = if !output.success && health == DiskHealthState::Unknown {
                    Some("smartctl returned non-zero status".to_string())
                } else {
                    None
                };

                (
                    DiskHealthDevice {
                        device: device.to_string(),
                        backend: "smartctl".to_string(),
                        health,
                        capability: Capability::available(),
                        temperature_c: None,
                        note,
                    },
                    Capability::available(),
                )
            }
            Err(capability) => (
                DiskHealthDevice {
                    device: device.to_string(),
                    backend: "smartctl".to_string(),
                    health: DiskHealthState::Unknown,
                    capability: capability.clone(),
                    temperature_c: None,
                    note: Some("smartctl unavailable".to_string()),
                },
                capability,
            ),
        }
    }

    fn probe_nvme_device(&self, device: &str) -> (DiskHealthDevice, Capability) {
        let dev_path = format!("/dev/{device}");
        let args = ["smart-log", dev_path.as_str(), "-o", "json"];

        match run_fixed_command(NVME_PATHS, &args) {
            Ok(output) => {
                let body = format!("{}\n{}", output.stdout, output.stderr);
                if is_permission_denied(&body) {
                    let capability = Capability::permission_denied(format!("nvme denied for {device}"));
                    return (
                        DiskHealthDevice {
                            device: device.to_string(),
                            backend: "nvme".to_string(),
                            health: DiskHealthState::Unknown,
                            capability: capability.clone(),
                            temperature_c: None,
                            note: Some("read-only probe blocked by permission".to_string()),
                        },
                        capability,
                    );
                }

                let parsed = serde_json::from_str::<Value>(&output.stdout).ok();
                let (health, temperature_c) = if let Some(value) = parsed {
                    let warning = value
                        .get("critical_warning")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    let temp_kelvin = value.get("temperature").and_then(Value::as_f64);
                    let temp_c = temp_kelvin.map(|k| (k - 273.15).max(-273.15));
                    let state = if warning == 0 {
                        DiskHealthState::Healthy
                    } else {
                        DiskHealthState::Degraded
                    };
                    (state, temp_c)
                } else {
                    (DiskHealthState::Unknown, None)
                };
                let is_unknown = health == DiskHealthState::Unknown;

                (
                    DiskHealthDevice {
                        device: device.to_string(),
                        backend: "nvme".to_string(),
                        health,
                        capability: Capability::available(),
                        temperature_c,
                        note: if is_unknown {
                            Some("nvme output parse failed".to_string())
                        } else {
                            None
                        },
                    },
                    Capability::available(),
                )
            }
            Err(capability) => (
                DiskHealthDevice {
                    device: device.to_string(),
                    backend: "nvme".to_string(),
                    health: DiskHealthState::Unknown,
                    capability: capability.clone(),
                    temperature_c: None,
                    note: Some("nvme tool unavailable".to_string()),
                },
                capability,
            ),
        }
    }
}

pub fn summarize_filesystem_samples(mut filesystems: Vec<FilesystemSample>) -> FilesystemStatus {
    filesystems.sort_by(|a, b| {
        b.used_pct
            .partial_cmp(&a.used_pct)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let full_filesystems = filesystems.iter().filter(|sample| sample.full).count();
    FilesystemStatus {
        filesystems,
        full_filesystems,
    }
}

pub fn pressure_value(pressure: &Option<MemoryPressure>) -> f64 {
    pressure
        .as_ref()
        .map(|sample| sample.some_avg10)
        .unwrap_or_default()
}

pub fn append_alerts(snapshot: &mut HealthSnapshot, active: Vec<AlertRecord>, log: Vec<AlertRecord>) {
    snapshot.alerts = active;
    snapshot.alert_log = log;
}

#[derive(Debug)]
struct FixedCommandOutput {
    stdout: String,
    stderr: String,
    success: bool,
}

fn run_fixed_command(paths: &[&str], args: &[&str]) -> Result<FixedCommandOutput, Capability> {
    let Some(path) = paths.iter().map(Path::new).find(|path| path.exists()) else {
        let joined = paths.join(",");
        return Err(Capability::missing_tool(joined));
    };

    let output = Command::new(path)
        .args(args)
        .output()
        .map_err(|err| Capability::error(err.to_string()))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if is_permission_denied(&stderr) || is_permission_denied(&stdout) {
        return Err(Capability::permission_denied(format!(
            "{} permission denied",
            path.display()
        )));
    }

    Ok(FixedCommandOutput {
        stdout,
        stderr,
        success: output.status.success(),
    })
}

fn resolve_tool_presence(paths: &[&str], tool_name: &str) -> Capability {
    if paths.iter().any(|path| Path::new(path).exists()) {
        Capability::available()
    } else {
        Capability::missing_tool(tool_name)
    }
}

fn list_block_devices(limit: usize) -> Vec<String> {
    let mut devices = Vec::new();
    if let Ok(entries) = fs::read_dir("/sys/block") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("loop") || name.starts_with("ram") {
                continue;
            }
            devices.push(name);
        }
    }

    devices.sort();
    if devices.len() > limit {
        devices.truncate(limit);
    }
    devices
}

fn pct(value: u64, total: u64) -> f64 {
    if total == 0 {
        return 0.0;
    }
    ((value as f64) / (total as f64) * 100.0).clamp(0.0, 100.0)
}

fn unix_now_ms() -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    now.as_millis().min(u128::from(u64::MAX)) as u64
}

fn should_skip_fs(fs_type: &str) -> bool {
    matches!(
        fs_type,
        "proc"
            | "sysfs"
            | "tmpfs"
            | "devtmpfs"
            | "devpts"
            | "cgroup"
            | "cgroup2"
            | "pstore"
            | "efivarfs"
            | "securityfs"
            | "overlay"
            | "squashfs"
            | "tracefs"
            | "configfs"
            | "ramfs"
            | "autofs"
            | "mqueue"
            | "debugfs"
            | "fusectl"
    )
}

fn unescape_mount(raw: &str) -> String {
    raw.replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\012", "\n")
        .replace("\\134", "\\")
}

fn counter_delta(current: u64, previous: u64) -> (u64, bool) {
    if current >= previous {
        (current - previous, false)
    } else {
        (0, true)
    }
}

fn read_link_speed_mbps(interface: &str) -> Option<u64> {
    let path = PathBuf::from("/sys/class/net").join(interface).join("speed");
    let value = fs::read_to_string(path).ok()?;
    let speed = value.trim().parse::<u64>().ok()?;
    if speed == u64::MAX {
        None
    } else {
        Some(speed)
    }
}

fn is_permission_denied(content: &str) -> bool {
    let lower = content.to_ascii_lowercase();
    lower.contains("permission denied") || lower.contains("operation not permitted")
}
