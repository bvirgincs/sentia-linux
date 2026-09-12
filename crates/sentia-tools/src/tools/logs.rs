use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::result::{PrivacyClass, Provenance, ToolResult};
use crate::runner::{first_available_executable, run_command};
use crate::validation::{
    bounded_limit, normalize_service_name, DEFAULT_LOG_LINES, DEFAULT_OUTPUT_LIMIT,
    DEFAULT_TIMEOUT_SECS, MAX_LOG_LINES,
};
use crate::ToolError;

const JOURNALCTL_CANDIDATES: &[&str] = &["/usr/bin/journalctl", "/bin/journalctl"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JournalRecentInput {
    pub lines: Option<usize>,
    pub unit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JournalErrorsInput {
    pub lines: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KernelLogsInput {
    pub lines: Option<usize>,
}

pub fn journal_recent(input: JournalRecentInput) -> Result<ToolResult, ToolError> {
    let lines = bounded_limit(input.lines, DEFAULT_LOG_LINES, MAX_LOG_LINES, "lines")?;

    let journalctl = first_available_executable(JOURNALCTL_CANDIDATES)
        .ok_or_else(|| ToolError::unavailable("journalctl is not available"))?;

    let mut args = vec![
        "--no-pager".to_string(),
        "--output=short-iso".to_string(),
        "--lines".to_string(),
        lines.to_string(),
    ];

    let normalized_unit = match input.unit.as_deref() {
        Some(unit) => {
            let normalized = normalize_service_name(unit)?;
            args.push("--unit".to_string());
            args.push(normalized.clone());
            Some(normalized)
        }
        None => None,
    };

    let output = run_command(
        journalctl,
        &args,
        DEFAULT_TIMEOUT_SECS,
        DEFAULT_OUTPUT_LIMIT,
        "journal_recent",
    )?;

    if output.status_code != 0 {
        return Err(classify_journal_failure(
            output.status_code,
            &output.stderr,
            "journal_recent",
        ));
    }

    let entries = collect_nonempty_lines(&output.stdout, lines);

    Ok(ToolResult::new(
        "journal_recent",
        PrivacyClass::PotentiallySensitive,
        Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        vec![Provenance {
            source: "subprocess".to_string(),
            detail: format!("{} --output=short-iso --lines N", journalctl),
        }],
        output.stdout_truncated || output.stderr_truncated || entries.len() >= lines,
        json!({
            "entries": entries,
            "lines": lines,
            "unit": normalized_unit,
        }),
    ))
}

pub fn journal_errors(input: JournalErrorsInput) -> Result<ToolResult, ToolError> {
    let lines = bounded_limit(input.lines, DEFAULT_LOG_LINES, MAX_LOG_LINES, "lines")?;

    let journalctl = first_available_executable(JOURNALCTL_CANDIDATES)
        .ok_or_else(|| ToolError::unavailable("journalctl is not available"))?;

    let args = vec![
        "--no-pager".to_string(),
        "--output=short-iso".to_string(),
        "--priority=3".to_string(),
        "--lines".to_string(),
        lines.to_string(),
    ];

    let output = run_command(
        journalctl,
        &args,
        DEFAULT_TIMEOUT_SECS,
        DEFAULT_OUTPUT_LIMIT,
        "journal_errors",
    )?;

    if output.status_code != 0 {
        return Err(classify_journal_failure(
            output.status_code,
            &output.stderr,
            "journal_errors",
        ));
    }

    let entries = collect_nonempty_lines(&output.stdout, lines);

    Ok(ToolResult::new(
        "journal_errors",
        PrivacyClass::PotentiallySensitive,
        Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        vec![Provenance {
            source: "subprocess".to_string(),
            detail: format!("{} --priority=3 --lines N", journalctl),
        }],
        output.stdout_truncated || output.stderr_truncated || entries.len() >= lines,
        json!({
            "entries": entries,
            "lines": lines,
        }),
    ))
}

pub fn kernel_logs(input: KernelLogsInput) -> Result<ToolResult, ToolError> {
    let lines = bounded_limit(input.lines, DEFAULT_LOG_LINES, MAX_LOG_LINES, "lines")?;

    let journalctl = first_available_executable(JOURNALCTL_CANDIDATES)
        .ok_or_else(|| ToolError::unavailable("journalctl is not available"))?;

    let args = vec![
        "--no-pager".to_string(),
        "--output=short-iso".to_string(),
        "--dmesg".to_string(),
        "--lines".to_string(),
        lines.to_string(),
    ];

    let output = run_command(
        journalctl,
        &args,
        DEFAULT_TIMEOUT_SECS,
        DEFAULT_OUTPUT_LIMIT,
        "kernel_logs",
    )?;

    if output.status_code != 0 {
        return Err(classify_journal_failure(
            output.status_code,
            &output.stderr,
            "kernel_logs",
        ));
    }

    let entries = collect_nonempty_lines(&output.stdout, lines);

    Ok(ToolResult::new(
        "kernel_logs",
        PrivacyClass::PotentiallySensitive,
        Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        vec![Provenance {
            source: "subprocess".to_string(),
            detail: format!("{} --dmesg --lines N", journalctl),
        }],
        output.stdout_truncated || output.stderr_truncated || entries.len() >= lines,
        json!({
            "entries": entries,
            "lines": lines,
        }),
    ))
}

fn collect_nonempty_lines(content: &str, max: usize) -> Vec<String> {
    content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(max)
        .map(ToOwned::to_owned)
        .collect()
}

fn classify_journal_failure(exit_code: i32, stderr: &str, operation: &str) -> ToolError {
    let lower = stderr.to_lowercase();

    if lower.contains("permission denied")
        || lower.contains("not permitted")
        || lower.contains("access denied")
    {
        return ToolError::permission_denied(stderr.trim().to_string());
    }

    if lower.contains("system has not been booted with systemd")
        || lower.contains("no journal files were found")
        || lower.contains("failed to connect to bus")
    {
        return ToolError::unavailable(format!("{operation} unavailable: {}", stderr.trim()));
    }

    ToolError::unavailable(format!(
        "journalctl failed for {operation} (exit {}): {}",
        exit_code,
        stderr.trim()
    ))
}
