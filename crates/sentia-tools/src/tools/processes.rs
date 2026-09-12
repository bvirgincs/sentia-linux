use std::fs;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::parsers::{parse_proc_stat_line, parse_proc_status_map};
use crate::result::{PrivacyClass, Provenance, ToolResult};
use crate::validation::{
    bounded_limit, read_text_file_bounded, DEFAULT_LIST_LIMIT, MAX_LIST_LIMIT,
};
use crate::ToolError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessListInput {
    pub limit: Option<usize>,
    #[serde(default)]
    pub include_cmdline: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessDetailsInput {
    pub pid: i32,
    #[serde(default)]
    pub include_cmdline: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessSummary {
    pub pid: i32,
    pub ppid: i32,
    pub name: String,
    pub state: String,
    pub uid: Option<u32>,
    pub vm_rss_kb: Option<u64>,
    pub threads: Option<u32>,
    pub cmdline: Option<String>,
}

pub fn process_list(input: ProcessListInput) -> Result<ToolResult, ToolError> {
    let limit = bounded_limit(input.limit, DEFAULT_LIST_LIMIT, MAX_LIST_LIMIT, "limit")?;

    let mut entries = Vec::<ProcessSummary>::new();
    let mut denied_count = 0_usize;
    let mut vanished_count = 0_usize;

    let proc_entries = fs::read_dir("/proc")
        .map_err(|err| ToolError::io("reading /proc", err.to_string()))?;

    for entry in proc_entries {
        let entry = entry.map_err(|err| ToolError::io("iterating /proc", err.to_string()))?;
        let file_name = entry.file_name();
        let Some(pid_str) = file_name.to_str() else {
            continue;
        };
        let Ok(pid) = pid_str.parse::<i32>() else {
            continue;
        };

        match read_process(pid, input.include_cmdline) {
            Ok(process) => entries.push(process),
            Err(ToolError::PermissionDenied { .. }) => denied_count += 1,
            Err(ToolError::NotFound { .. }) => vanished_count += 1,
            Err(ToolError::Io { message, .. }) if message.contains("No such file") => {
                vanished_count += 1
            }
            Err(_) => continue,
        }
    }

    entries.sort_by(|a, b| a.pid.cmp(&b.pid));

    let partial = entries.len() > limit;
    if entries.len() > limit {
        entries.truncate(limit);
    }

    Ok(ToolResult::new(
        "process_list",
        PrivacyClass::PotentiallySensitive,
        Duration::from_secs(2),
        vec![Provenance {
            source: "procfs".to_string(),
            detail: "/proc/<pid>/stat, /proc/<pid>/status, optional /proc/<pid>/cmdline".to_string(),
        }],
        partial,
        json!({
            "processes": entries,
            "limit": limit,
            "include_cmdline": input.include_cmdline,
            "permission_denied_count": denied_count,
            "vanished_count": vanished_count,
        }),
    ))
}

pub fn process_details(input: ProcessDetailsInput) -> Result<ToolResult, ToolError> {
    if input.pid <= 0 {
        return Err(ToolError::invalid_input("pid must be a positive integer"));
    }

    let process = read_process(input.pid, input.include_cmdline)?;

    Ok(ToolResult::new(
        "process_details",
        PrivacyClass::PotentiallySensitive,
        Duration::from_secs(2),
        vec![Provenance {
            source: "procfs".to_string(),
            detail: "/proc/<pid>/stat, /proc/<pid>/status, optional /proc/<pid>/cmdline".to_string(),
        }],
        false,
        json!({
            "process": process,
        }),
    ))
}

fn read_process(pid: i32, include_cmdline: bool) -> Result<ProcessSummary, ToolError> {
    let base = Path::new("/proc").join(pid.to_string());

    let stat = read_text_file_bounded(&base.join("stat"), 16 * 1024).map_err(|err| {
        annotate_process_error(err, pid, "stat")
    })?;
    let parsed_stat = parse_proc_stat_line(stat.trim())?;

    let status_content = read_text_file_bounded(&base.join("status"), 64 * 1024)
        .map_err(|err| annotate_process_error(err, pid, "status"))?;
    let status = parse_proc_status_map(&status_content);

    let uid = status
        .get("Uid")
        .and_then(|value| value.split_whitespace().next())
        .and_then(|value| value.parse::<u32>().ok());

    let vm_rss_kb = status
        .get("VmRSS")
        .and_then(|value| value.split_whitespace().next())
        .and_then(|value| value.parse::<u64>().ok());

    let threads = status
        .get("Threads")
        .and_then(|value| value.parse::<u32>().ok());

    let cmdline = if include_cmdline {
        match fs::read(base.join("cmdline")) {
            Ok(content) => {
                let rendered = content
                    .split(|byte| *byte == 0)
                    .filter(|part| !part.is_empty())
                    .map(|part| String::from_utf8_lossy(part).to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                if rendered.is_empty() {
                    None
                } else {
                    Some(rendered.chars().take(8_192).collect())
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
                return Err(ToolError::permission_denied(format!(
                    "permission denied reading /proc/{pid}/cmdline"
                )));
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
            Err(err) => {
                return Err(ToolError::io(
                    format!("reading /proc/{pid}/cmdline"),
                    err.to_string(),
                ));
            }
        }
    } else {
        None
    };

    Ok(ProcessSummary {
        pid: parsed_stat.pid,
        ppid: parsed_stat.ppid,
        name: parsed_stat.name,
        state: parsed_stat.state,
        uid,
        vm_rss_kb,
        threads,
        cmdline,
    })
}

fn annotate_process_error(error: ToolError, pid: i32, file: &str) -> ToolError {
    match error {
        ToolError::PermissionDenied { .. } => ToolError::permission_denied(format!(
            "permission denied reading /proc/{pid}/{file}"
        )),
        ToolError::Io { context, message } => {
            if message.contains("No such file") {
                ToolError::not_found(format!("process {pid} vanished while reading {file}"))
            } else {
                ToolError::io(context, message)
            }
        }
        other => other,
    }
}
