use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::PathBuf;

use crate::terminal::AssistanceMode;
use crate::transport::RouterPolicy;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivacyCategories {
    pub files: bool,
    pub command_history: bool,
    pub hostnames: bool,
    pub ip_addresses: bool,
    pub process_names: bool,
    pub journals: bool,
}

impl Default for PrivacyCategories {
    fn default() -> Self {
        Self {
            files: false,
            command_history: false,
            hostnames: false,
            ip_addresses: false,
            process_names: false,
            journals: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirstBootConfig {
    pub wizard_skipped: bool,
    pub completed: bool,
    pub policy: RouterPolicy,
    pub telemetry_enabled: bool,
    pub terminal_mode: AssistanceMode,
    pub privacy: PrivacyCategories,
    pub eligible_providers: Vec<String>,
    pub updated_at: String,
}

impl Default for FirstBootConfig {
    fn default() -> Self {
        Self {
            wizard_skipped: false,
            completed: false,
            policy: RouterPolicy::LocalOnly,
            telemetry_enabled: false,
            terminal_mode: AssistanceMode::CommandNotFound,
            privacy: PrivacyCategories::default(),
            eligible_providers: Vec::new(),
            updated_at: now_timestamp(),
        }
    }
}

impl FirstBootConfig {
    pub fn load_or_default(path: Option<PathBuf>) -> Self {
        let path = path.unwrap_or_else(default_config_path);

        let Ok(raw) = fs::read_to_string(path) else {
            return Self::default();
        };

        serde_json::from_str(&raw).unwrap_or_else(|_| Self::default())
    }

    pub fn save(&self, path: Option<PathBuf>) -> std::io::Result<PathBuf> {
        let path = path.unwrap_or_else(default_config_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let payload = serde_json::to_string_pretty(self)
            .expect("serializing firstboot config should be infallible");
        fs::write(&path, payload)?;

        Ok(path)
    }
}

pub fn default_config_path() -> PathBuf {
    if let Some(path) = env::var_os("SENTIA_FIRSTBOOT_CONFIG") {
        return PathBuf::from(path);
    }

    let base = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"));

    base.join("sentia/firstboot.json")
}

fn now_timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    format!("unix:{now}")
}
