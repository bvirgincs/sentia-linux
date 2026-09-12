use std::collections::{HashMap, VecDeque};

use crate::collector::pressure_value;
use crate::schema::{AlertLevel, AlertRecord, DiskHealthState, HealthSnapshot};

#[derive(Debug, Clone)]
struct ThresholdRule {
    activate_at: f64,
    clear_below: f64,
    required_samples: usize,
    significant_delta: f64,
    level: AlertLevel,
}

#[derive(Debug, Clone)]
struct ConditionState {
    active: bool,
    above_samples: usize,
    below_samples: usize,
    first_seen_unix_ms: u64,
    last_seen_unix_ms: u64,
    last_emitted_value: f64,
    message: String,
    level: AlertLevel,
}

#[derive(Debug)]
pub struct AlertEngine {
    states: HashMap<String, ConditionState>,
    log: VecDeque<AlertRecord>,
    max_log_entries: usize,
}

impl AlertEngine {
    pub fn new(max_log_entries: usize) -> Self {
        Self {
            states: HashMap::new(),
            log: VecDeque::with_capacity(max_log_entries),
            max_log_entries,
        }
    }

    pub fn evaluate(&mut self, snapshot: &HealthSnapshot) -> (Vec<AlertRecord>, Vec<AlertRecord>) {
        let now = snapshot.collected_at_unix_ms;

        self.eval_numeric(
            "cpu_high",
            snapshot.cpu.utilization_pct,
            ThresholdRule {
                activate_at: 90.0,
                clear_below: 80.0,
                required_samples: 2,
                significant_delta: 5.0,
                level: AlertLevel::Warning,
            },
            now,
            |v| format!("CPU utilization sustained at {:.1}%", v),
        );

        self.eval_numeric(
            "memory_high",
            snapshot.memory.used_pct,
            ThresholdRule {
                activate_at: 92.0,
                clear_below: 86.0,
                required_samples: 2,
                significant_delta: 3.0,
                level: AlertLevel::Warning,
            },
            now,
            |v| format!("Memory usage sustained at {:.1}%", v),
        );

        self.eval_numeric(
            "swap_high",
            snapshot.swap.used_pct,
            ThresholdRule {
                activate_at: 85.0,
                clear_below: 70.0,
                required_samples: 2,
                significant_delta: 5.0,
                level: AlertLevel::Warning,
            },
            now,
            |v| format!("Swap usage sustained at {:.1}%", v),
        );

        self.eval_numeric(
            "filesystem_full",
            snapshot.filesystem.full_filesystems as f64,
            ThresholdRule {
                activate_at: 1.0,
                clear_below: 0.5,
                required_samples: 1,
                significant_delta: 1.0,
                level: AlertLevel::Critical,
            },
            now,
            |v| format!("{} filesystem(s) are full or nearly full", v as usize),
        );

        self.eval_numeric(
            "memory_pressure",
            pressure_value(&snapshot.memory.pressure),
            ThresholdRule {
                activate_at: 1.0,
                clear_below: 0.3,
                required_samples: 3,
                significant_delta: 0.5,
                level: AlertLevel::Warning,
            },
            now,
            |v| format!("Memory PSI some avg10 elevated ({:.2})", v),
        );

        self.eval_numeric(
            "failed_units",
            snapshot.systemd.failed_units.len() as f64,
            ThresholdRule {
                activate_at: 1.0,
                clear_below: 0.5,
                required_samples: 1,
                significant_delta: 1.0,
                level: AlertLevel::Warning,
            },
            now,
            |v| format!("{} failed systemd unit(s) detected", v as usize),
        );

        self.eval_numeric(
            "kernel_errors",
            snapshot.kernel.recent_errors.len() as f64,
            ThresholdRule {
                activate_at: 1.0,
                clear_below: 0.5,
                required_samples: 1,
                significant_delta: 2.0,
                level: AlertLevel::Warning,
            },
            now,
            |v| format!("{} recent kernel error(s) reported", v as usize),
        );

        let disk_health_flag = if snapshot.diskhealth.overall == DiskHealthState::Degraded {
            1.0
        } else {
            0.0
        };

        self.eval_numeric(
            "disk_health",
            disk_health_flag,
            ThresholdRule {
                activate_at: 0.5,
                clear_below: 0.5,
                required_samples: 1,
                significant_delta: 1.0,
                level: AlertLevel::Critical,
            },
            now,
            |_v| "Disk health reported degraded".to_string(),
        );

        let max_temp = snapshot.temperature.max_c.unwrap_or_default();
        self.eval_numeric(
            "temperature_high",
            max_temp,
            ThresholdRule {
                activate_at: 85.0,
                clear_below: 78.0,
                required_samples: 2,
                significant_delta: 2.0,
                level: AlertLevel::Warning,
            },
            now,
            |v| format!("Temperature sustained at {:.1}°C", v),
        );

        self.eval_numeric(
            "process_churn",
            snapshot.processes.churned_processes as f64,
            ThresholdRule {
                activate_at: 64.0,
                clear_below: 16.0,
                required_samples: 2,
                significant_delta: 16.0,
                level: AlertLevel::Info,
            },
            now,
            |v| format!("High process churn observed ({} process delta)", v as usize),
        );

        let mut active = Vec::new();
        for (id, state) in &self.states {
            if !state.active {
                continue;
            }
            active.push(AlertRecord {
                id: id.clone(),
                level: state.level.clone(),
                active: true,
                message: state.message.clone(),
                value: state.last_emitted_value,
                first_seen_unix_ms: state.first_seen_unix_ms,
                last_seen_unix_ms: state.last_seen_unix_ms,
            });
        }

        active.sort_by(|a, b| a.id.cmp(&b.id));
        (active, self.log.iter().cloned().collect())
    }

    fn eval_numeric<F>(
        &mut self,
        id: &str,
        value: f64,
        rule: ThresholdRule,
        now: u64,
        message_fn: F,
    ) where
        F: Fn(f64) -> String,
    {
        let mut log_record: Option<AlertRecord> = None;
        {
            let state = self
                .states
                .entry(id.to_string())
                .or_insert_with(|| ConditionState {
                    active: false,
                    above_samples: 0,
                    below_samples: 0,
                    first_seen_unix_ms: now,
                    last_seen_unix_ms: now,
                    last_emitted_value: value,
                    message: String::new(),
                    level: rule.level.clone(),
                });

            state.level = rule.level.clone();

            if state.active {
                let below_clear_threshold = value <= rule.clear_below;
                if below_clear_threshold {
                    state.below_samples = state.below_samples.saturating_add(1);
                } else {
                    state.below_samples = 0;
                }

                if state.below_samples >= rule.required_samples {
                    state.active = false;
                    state.above_samples = 0;
                    state.last_seen_unix_ms = now;
                    state.message = format!("{} (resolved)", message_fn(value));
                    state.last_emitted_value = value;
                    log_record = Some(AlertRecord {
                        id: id.to_string(),
                        level: state.level.clone(),
                        active: false,
                        message: state.message.clone(),
                        value: state.last_emitted_value,
                        first_seen_unix_ms: state.first_seen_unix_ms,
                        last_seen_unix_ms: state.last_seen_unix_ms,
                    });
                } else if !below_clear_threshold
                    && (value - state.last_emitted_value).abs() >= rule.significant_delta
                {
                    state.message = message_fn(value);
                    state.last_emitted_value = value;
                    state.last_seen_unix_ms = now;
                    log_record = Some(AlertRecord {
                        id: id.to_string(),
                        level: state.level.clone(),
                        active: true,
                        message: state.message.clone(),
                        value: state.last_emitted_value,
                        first_seen_unix_ms: state.first_seen_unix_ms,
                        last_seen_unix_ms: state.last_seen_unix_ms,
                    });
                }
            } else if value >= rule.activate_at {
                state.above_samples = state.above_samples.saturating_add(1);
                if state.above_samples >= rule.required_samples {
                    state.active = true;
                    state.below_samples = 0;
                    state.first_seen_unix_ms = now;
                    state.last_seen_unix_ms = now;
                    state.message = message_fn(value);
                    state.last_emitted_value = value;
                    log_record = Some(AlertRecord {
                        id: id.to_string(),
                        level: state.level.clone(),
                        active: true,
                        message: state.message.clone(),
                        value: state.last_emitted_value,
                        first_seen_unix_ms: state.first_seen_unix_ms,
                        last_seen_unix_ms: state.last_seen_unix_ms,
                    });
                }
            } else {
                state.above_samples = 0;
            }
        }

        if let Some(record) = log_record {
            self.push_record(record);
        }
    }

    fn push_record(&mut self, record: AlertRecord) {
        if self.max_log_entries == 0 {
            return;
        }

        if self.log.len() >= self.max_log_entries {
            self.log.pop_front();
        }
        self.log.push_back(record);
    }
}

impl Default for AlertEngine {
    fn default() -> Self {
        Self::new(256)
    }
}
