use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::result::{PrivacyClass, Provenance, ToolResult};
use crate::runner::{first_available_executable, run_command};
use crate::validation::{
    bounded_limit, normalize_service_name, DEFAULT_LIST_LIMIT, DEFAULT_OUTPUT_LIMIT,
    DEFAULT_TIMEOUT_SECS, MAX_LIST_LIMIT,
};
use crate::ToolError;

const SYSTEMCTL_CANDIDATES: &[&str] = &["/bin/systemctl", "/usr/bin/systemctl"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceListInput {
    pub limit: Option<usize>,
    #[serde(default = "default_true")]
    pub include_inactive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceStatusInput {
    pub service: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceUnitSummary {
    pub unit: String,
    pub load_state: String,
    pub active_state: String,
    pub sub_state: String,
    pub description: String,
}

pub fn service_list(input: ServiceListInput) -> Result<ToolResult, ToolError> {
    let limit = bounded_limit(input.limit, DEFAULT_LIST_LIMIT, MAX_LIST_LIMIT, "limit")?;

    let systemctl = first_available_executable(SYSTEMCTL_CANDIDATES)
        .ok_or_else(|| ToolError::unavailable("systemctl is not available"))?;

    let mut args = vec![
        "list-units".to_string(),
        "--type=service".to_string(),
        "--no-pager".to_string(),
        "--no-legend".to_string(),
        "--plain".to_string(),
    ];
    if input.include_inactive {
        args.push("--all".to_string());
    }

    let output = run_command(
        systemctl,
        &args,
        DEFAULT_TIMEOUT_SECS,
        DEFAULT_OUTPUT_LIMIT,
        "service_list",
    )?;

    if output.status_code != 0 {
        return Err(classify_systemctl_failure(
            output.status_code,
            &output.stderr,
            "service listing",
        ));
    }

    let mut units = Vec::new();
    for line in output.stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let columns: Vec<&str> = trimmed.split_whitespace().collect();
        if columns.len() < 5 {
            continue;
        }

        units.push(ServiceUnitSummary {
            unit: columns[0].to_string(),
            load_state: columns[1].to_string(),
            active_state: columns[2].to_string(),
            sub_state: columns[3].to_string(),
            description: columns[4..].join(" "),
        });

        if units.len() >= limit {
            break;
        }
    }

    Ok(ToolResult::new(
        "service_list",
        PrivacyClass::SystemMetadata,
        Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        vec![Provenance {
            source: "subprocess".to_string(),
            detail: format!("{} list-units --type=service ...", systemctl),
        }],
        output.stdout_truncated || output.stderr_truncated || units.len() >= limit,
        json!({
            "services": units,
            "include_inactive": input.include_inactive,
            "limit": limit,
        }),
    ))
}

pub fn service_status(input: ServiceStatusInput) -> Result<ToolResult, ToolError> {
    let service = normalize_service_name(&input.service)?;

    let systemctl = first_available_executable(SYSTEMCTL_CANDIDATES)
        .ok_or_else(|| ToolError::unavailable("systemctl is not available"))?;

    let args = vec![
        "show".to_string(),
        "--no-pager".to_string(),
        "--property=Id,Description,LoadState,ActiveState,SubState,UnitFileState,MainPID,ExecMainStatus,FragmentPath".to_string(),
        service.clone(),
    ];

    let output = run_command(
        systemctl,
        &args,
        DEFAULT_TIMEOUT_SECS,
        DEFAULT_OUTPUT_LIMIT,
        "service_status",
    )?;

    if output.status_code != 0 {
        return Err(classify_systemctl_failure(
            output.status_code,
            &output.stderr,
            &format!("service status for {service}"),
        ));
    }

    let mut status = BTreeMap::<String, String>::new();
    for line in output.stdout.lines() {
        if let Some((k, v)) = line.split_once('=') {
            status.insert(k.to_string(), v.to_string());
        }
    }

    if status.is_empty() {
        return Err(ToolError::parse(
            "systemctl show",
            "empty service status output",
        ));
    }

    Ok(ToolResult::new(
        "service_status",
        PrivacyClass::SystemMetadata,
        Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        vec![Provenance {
            source: "subprocess".to_string(),
            detail: format!("{} show --property=... <service>", systemctl),
        }],
        output.stdout_truncated || output.stderr_truncated,
        json!({
            "service": service,
            "status": status,
        }),
    ))
}

fn classify_systemctl_failure(exit_code: i32, stderr: &str, context: &str) -> ToolError {
    let lower = stderr.to_lowercase();
    if lower.contains("system has not been booted with systemd")
        || lower.contains("failed to connect to bus")
        || lower.contains("no such file or directory")
    {
        return ToolError::unavailable(format!("{context} unavailable: {}", stderr.trim()));
    }

    if lower.contains("access denied") || lower.contains("permission denied") {
        return ToolError::permission_denied(stderr.trim().to_string());
    }

    if lower.contains("not found") || lower.contains("could not be found") {
        return ToolError::not_found(stderr.trim().to_string());
    }

    ToolError::unavailable(format!(
        "systemctl failed for {context} (exit {}): {}",
        exit_code,
        stderr.trim()
    ))
}

fn default_true() -> bool {
    true
}
