use std::collections::HashMap;

use crate::schema::MemoryPressure;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuTimes {
    pub user: u64,
    pub nice: u64,
    pub system: u64,
    pub idle: u64,
    pub iowait: u64,
    pub irq: u64,
    pub softirq: u64,
    pub steal: u64,
}

impl CpuTimes {
    pub fn active(self) -> u64 {
        self.user
            .saturating_add(self.nice)
            .saturating_add(self.system)
            .saturating_add(self.irq)
            .saturating_add(self.softirq)
            .saturating_add(self.steal)
    }

    pub fn total(self) -> u64 {
        self.active()
            .saturating_add(self.idle)
            .saturating_add(self.iowait)
    }

    pub fn delta(self, previous: Self) -> Option<CpuDelta> {
        let total_now = self.total();
        let total_prev = previous.total();
        let active_now = self.active();
        let active_prev = previous.active();

        if total_now < total_prev || active_now < active_prev {
            return None;
        }

        Some(CpuDelta {
            total: total_now - total_prev,
            active: active_now - active_prev,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuDelta {
    pub total: u64,
    pub active: u64,
}

pub fn parse_cpu_total(line_data: &str) -> Option<CpuTimes> {
    let line = line_data.lines().find(|line| line.starts_with("cpu "))?;
    let mut parts = line.split_whitespace();
    parts.next()?;

    let mut values = [0_u64; 8];
    for (idx, value) in parts.take(8).enumerate() {
        values[idx] = value.parse().ok()?;
    }

    Some(CpuTimes {
        user: values[0],
        nice: values[1],
        system: values[2],
        idle: values[3],
        iowait: values[4],
        irq: values[5],
        softirq: values[6],
        steal: values[7],
    })
}

pub fn cpu_utilization_pct(current: CpuTimes, previous: CpuTimes) -> Option<f64> {
    let delta = current.delta(previous)?;
    if delta.total == 0 {
        return Some(0.0);
    }

    Some(((delta.active as f64) / (delta.total as f64) * 100.0).clamp(0.0, 100.0))
}

pub fn parse_loadavg(content: &str) -> Option<(f64, f64, f64)> {
    let mut parts = content.split_whitespace();
    let load_1 = parts.next()?.parse().ok()?;
    let load_5 = parts.next()?.parse().ok()?;
    let load_15 = parts.next()?.parse().ok()?;
    Some((load_1, load_5, load_15))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemInfo {
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub free_bytes: u64,
    pub buffers_bytes: u64,
    pub cached_bytes: u64,
    pub swap_total_bytes: u64,
    pub swap_free_bytes: u64,
}

pub fn parse_meminfo(content: &str) -> Option<MemInfo> {
    let mut values_kb: HashMap<&str, u64> = HashMap::new();

    for line in content.lines() {
        let mut parts = line.split_whitespace();
        let raw_key = parts.next()?;
        let key = raw_key.trim_end_matches(':');
        let value = parts.next()?.parse::<u64>().ok()?;
        values_kb.insert(key, value);
    }

    let total = values_kb.get("MemTotal")?.saturating_mul(1024);
    let available = values_kb
        .get("MemAvailable")
        .copied()
        .or_else(|| {
            values_kb
                .get("MemFree")
                .zip(values_kb.get("Buffers"))
                .map(|(free, buffers)| free.saturating_add(*buffers))
        })?
        .saturating_mul(1024);

    Some(MemInfo {
        total_bytes: total,
        available_bytes: available,
        free_bytes: values_kb.get("MemFree").copied().unwrap_or_default() * 1024,
        buffers_bytes: values_kb.get("Buffers").copied().unwrap_or_default() * 1024,
        cached_bytes: values_kb.get("Cached").copied().unwrap_or_default() * 1024,
        swap_total_bytes: values_kb.get("SwapTotal").copied().unwrap_or_default() * 1024,
        swap_free_bytes: values_kb.get("SwapFree").copied().unwrap_or_default() * 1024,
    })
}

pub fn parse_memory_pressure(content: &str) -> Option<MemoryPressure> {
    let mut some_values: HashMap<&str, f64> = HashMap::new();
    let mut full_values: HashMap<&str, f64> = HashMap::new();

    for line in content.lines() {
        let mut parts = line.split_whitespace();
        let category = parts.next()?;
        let target = if category == "some" {
            &mut some_values
        } else if category == "full" {
            &mut full_values
        } else {
            continue;
        };

        for segment in parts {
            let (key, value) = segment.split_once('=')?;
            if key == "avg10" || key == "avg60" || key == "avg300" {
                target.insert(key, value.parse().ok()?);
            }
        }
    }

    Some(MemoryPressure {
        some_avg10: *some_values.get("avg10")?,
        some_avg60: *some_values.get("avg60")?,
        some_avg300: *some_values.get("avg300")?,
        full_avg10: *full_values.get("avg10")?,
        full_avg60: *full_values.get("avg60")?,
        full_avg300: *full_values.get("avg300")?,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetCounters {
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

pub fn parse_net_dev(content: &str) -> HashMap<String, NetCounters> {
    let mut out = HashMap::new();

    for line in content.lines().skip(2) {
        let Some((iface, data)) = line.split_once(':') else {
            continue;
        };

        let fields: Vec<&str> = data.split_whitespace().collect();
        if fields.len() < 16 {
            continue;
        }

        let rx_bytes = fields[0].parse::<u64>().ok();
        let tx_bytes = fields[8].parse::<u64>().ok();

        if let (Some(rx), Some(tx)) = (rx_bytes, tx_bytes) {
            out.insert(
                iface.trim().to_string(),
                NetCounters {
                    rx_bytes: rx,
                    tx_bytes: tx,
                },
            );
        }
    }

    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskCounters {
    pub read_bytes: u64,
    pub write_bytes: u64,
    pub io_time_ms: u64,
}

pub fn parse_diskstats(content: &str) -> HashMap<String, DiskCounters> {
    let mut out = HashMap::new();

    for line in content.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 14 {
            continue;
        }

        let name = fields[2];
        if !is_primary_block_device(name) {
            continue;
        }

        let sectors_read = fields[5].parse::<u64>().ok();
        let sectors_written = fields[9].parse::<u64>().ok();
        let io_time_ms = fields[12].parse::<u64>().ok();

        if let (Some(read), Some(write), Some(io_time)) = (sectors_read, sectors_written, io_time_ms) {
            out.insert(
                name.to_string(),
                DiskCounters {
                    read_bytes: read.saturating_mul(512),
                    write_bytes: write.saturating_mul(512),
                    io_time_ms: io_time,
                },
            );
        }
    }

    out
}

pub fn is_primary_block_device(device: &str) -> bool {
    if device.starts_with("loop") || device.starts_with("ram") {
        return false;
    }

    if device.starts_with("nvme") {
        return !device.contains('p');
    }

    if device.starts_with("mmcblk") {
        return !device.contains('p');
    }

    !device.chars().last().is_some_and(|ch| ch.is_ascii_digit())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessStat {
    pub pid: u32,
    pub name: String,
    pub cpu_jiffies: u64,
    pub rss_pages: i64,
}

pub fn parse_process_stat(content: &str) -> Option<ProcessStat> {
    let open = content.find('(')?;
    let close = content.rfind(')')?;

    let pid = content[..open].trim().parse::<u32>().ok()?;
    let name = content[(open + 1)..close].to_string();

    let rest = content[(close + 1)..].trim();
    let fields: Vec<&str> = rest.split_whitespace().collect();
    if fields.len() < 22 {
        return None;
    }

    let utime = fields[11].parse::<u64>().ok()?;
    let stime = fields[12].parse::<u64>().ok()?;
    let rss_pages = fields[21].parse::<i64>().ok()?;

    Some(ProcessStat {
        pid,
        name,
        cpu_jiffies: utime.saturating_add(stime),
        rss_pages,
    })
}

pub fn sanitize_log_message(raw: &str, max_len: usize) -> String {
    let mut out = String::with_capacity(raw.len().min(max_len));

    for token in raw.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        let redacted = if let Some((key, value)) = token.split_once('=') {
            let key_lower = key.to_ascii_lowercase();
            if key_lower.contains("token")
                || key_lower.contains("secret")
                || key_lower.contains("password")
                || key_lower.contains("passwd")
                || key_lower.contains("key")
            {
                format!("{}=<redacted>", key)
            } else {
                let should_redact = value.len() > 48 && value.chars().all(|ch| ch.is_ascii_alphanumeric());
                if should_redact {
                    format!("{}=<redacted>", key)
                } else {
                    token.to_string()
                }
            }
        } else if token.len() > 64 && lower.chars().all(|ch| ch.is_ascii_alphanumeric()) {
            "<redacted>".to_string()
        } else {
            token.to_string()
        };

        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&redacted);

        if out.len() >= max_len {
            out.truncate(max_len);
            return out;
        }
    }

    out
}
