use std::time::Duration;

use crate::alert::AlertEngine;
use crate::collector::{append_alerts, CollectorConfig, HealthCollector};
use crate::schema::HealthSnapshot;

#[derive(Debug, Clone, Copy)]
pub struct AdaptiveIntervals {
    pub min: Duration,
    pub normal: Duration,
    pub max: Duration,
}

impl Default for AdaptiveIntervals {
    fn default() -> Self {
        Self {
            min: Duration::from_secs(2),
            normal: Duration::from_secs(5),
            max: Duration::from_secs(20),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MonitorConfig {
    pub collector: CollectorConfig,
    pub intervals: AdaptiveIntervals,
    pub alert_log_capacity: usize,
}

impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            collector: CollectorConfig::default(),
            intervals: AdaptiveIntervals::default(),
            alert_log_capacity: 256,
        }
    }
}

#[derive(Debug)]
pub struct HealthMonitor {
    collector: HealthCollector,
    alerts: AlertEngine,
    config: MonitorConfig,
    stable_ticks: usize,
    next_interval: Duration,
    previous: Option<HealthSnapshot>,
}

impl HealthMonitor {
    pub fn new(config: MonitorConfig) -> Self {
        let interval = config.intervals.normal;
        Self {
            collector: HealthCollector::new(config.collector.clone()),
            alerts: AlertEngine::new(config.alert_log_capacity),
            config,
            stable_ticks: 0,
            next_interval: interval,
            previous: None,
        }
    }

    pub fn sample(&mut self) -> HealthSnapshot {
        let mut snapshot = self.collector.collect();
        let (active_alerts, log) = self.alerts.evaluate(&snapshot);
        append_alerts(&mut snapshot, active_alerts, log);

        self.next_interval = self.compute_interval(&snapshot);
        self.previous = Some(snapshot.clone());
        snapshot
    }

    pub fn next_interval(&self) -> Duration {
        self.next_interval
    }

    fn compute_interval(&mut self, current: &HealthSnapshot) -> Duration {
        let has_active_alerts = !current.alerts.is_empty();

        if has_active_alerts {
            self.stable_ticks = 0;
            return self.config.intervals.min;
        }

        let significant_change = self
            .previous
            .as_ref()
            .map(|previous| is_significant_change(previous, current))
            .unwrap_or(true);

        if significant_change {
            self.stable_ticks = 0;
            self.config.intervals.normal
        } else {
            self.stable_ticks = self.stable_ticks.saturating_add(1);
            if self.stable_ticks >= 5 {
                self.config.intervals.max
            } else {
                self.config.intervals.normal
            }
        }
    }
}

impl Default for HealthMonitor {
    fn default() -> Self {
        Self::new(MonitorConfig::default())
    }
}

fn is_significant_change(previous: &HealthSnapshot, current: &HealthSnapshot) -> bool {
    let cpu_delta = (previous.cpu.utilization_pct - current.cpu.utilization_pct).abs();
    if cpu_delta >= 10.0 {
        return true;
    }

    let memory_delta = (previous.memory.used_pct - current.memory.used_pct).abs();
    if memory_delta >= 5.0 {
        return true;
    }

    let net_delta = ((previous.network.total_rx_bps + previous.network.total_tx_bps)
        - (current.network.total_rx_bps + current.network.total_tx_bps))
        .abs();
    if net_delta >= 5_000_000.0 {
        return true;
    }

    let disk_delta =
        ((previous.disk.total_read_bps + previous.disk.total_write_bps)
            - (current.disk.total_read_bps + current.disk.total_write_bps))
            .abs();
    if disk_delta >= 10_000_000.0 {
        return true;
    }

    previous.filesystem.full_filesystems != current.filesystem.full_filesystems
        || previous.systemd.failed_units.len() != current.systemd.failed_units.len()
        || previous.kernel.recent_errors.len() != current.kernel.recent_errors.len()
}
