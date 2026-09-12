use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::result::{PrivacyClass, Provenance, ToolResult};
use crate::runner::{first_available_executable, run_command};
use crate::validation::{
    bounded_limit, validate_command_token, DEFAULT_HISTORY_LIMIT, DEFAULT_OUTPUT_LIMIT,
    DEFAULT_TIMEOUT_SECS, MAX_HISTORY_ENTRIES, MAX_HISTORY_LIMIT,
};
use crate::ToolError;

const SAFE_COMMAND_PATHS: [&str; 6] = [
    "/usr/local/sbin",
    "/usr/local/bin",
    "/usr/sbin",
    "/usr/bin",
    "/sbin",
    "/bin",
];

const MAN_CANDIDATES: &[&str] = &["/usr/bin/man", "/bin/man"];
const DPKG_QUERY_CANDIDATES: &[&str] = &["/usr/bin/dpkg-query", "/bin/dpkg-query"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandExistsInput {
    pub command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandLookupInput {
    pub command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandHelpInput {
    pub command: String,
    pub section: Option<String>,
    pub max_chars: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandHistoryEntry {
    pub command: String,
    pub cwd: Option<PathBuf>,
    pub exit_code: Option<i32>,
    pub timestamp: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandHistoryInput {
    pub entries: Vec<CommandHistoryEntry>,
    pub query: Option<String>,
    pub limit: Option<usize>,
    #[serde(default)]
    pub failures_only: bool,
}

pub fn command_exists(input: CommandExistsInput) -> Result<ToolResult, ToolError> {
    validate_command_token(&input.command)?;

    let path = lookup_command_path(&input.command);

    Ok(ToolResult::new(
        "command_exists",
        PrivacyClass::PublicMetadata,
        Duration::from_secs(1),
        vec![Provenance {
            source: "filesystem".to_string(),
            detail: "searched fixed executable paths".to_string(),
        }],
        false,
        json!({
            "command": input.command,
            "exists": path.is_some(),
            "path": path,
            "searched_paths": SAFE_COMMAND_PATHS,
        }),
    ))
}

pub fn command_lookup(input: CommandLookupInput) -> Result<ToolResult, ToolError> {
    validate_command_token(&input.command)?;

    let path = lookup_command_path(&input.command)
        .ok_or_else(|| ToolError::not_found(format!("command '{}' not found", input.command)))?;

    Ok(ToolResult::new(
        "command_lookup",
        PrivacyClass::PublicMetadata,
        Duration::from_secs(1),
        vec![Provenance {
            source: "filesystem".to_string(),
            detail: "searched fixed executable paths".to_string(),
        }],
        false,
        json!({
            "command": input.command,
            "path": path,
            "searched_paths": SAFE_COMMAND_PATHS,
        }),
    ))
}

pub fn command_help(input: CommandHelpInput) -> Result<ToolResult, ToolError> {
    validate_command_token(&input.command)?;
    validate_optional_man_section(input.section.as_deref())?;

    let max_chars = bounded_limit(input.max_chars, 12_000, 60_000, "max_chars")?;
    let max_output = max_chars.saturating_mul(2).clamp(8_192, DEFAULT_OUTPUT_LIMIT * 4);

    let command_path = lookup_command_path(&input.command)
        .ok_or_else(|| ToolError::not_found(format!("command '{}' not found", input.command)))?;

    let man_exe = first_available_executable(MAN_CANDIDATES)
        .ok_or_else(|| ToolError::unavailable("man executable not available"))?;

    let where_args = build_man_args("--where", input.section.as_deref(), &input.command);
    let man_where = run_command(
        man_exe,
        &where_args,
        DEFAULT_TIMEOUT_SECS,
        DEFAULT_OUTPUT_LIMIT,
        "command_help:man_where",
    )?;

    let man_locations = if man_where.status_code == 0 {
        man_where
            .stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    let render_args = build_man_args("--pager=cat", input.section.as_deref(), &input.command);
    let man_render = run_command(
        man_exe,
        &render_args,
        DEFAULT_TIMEOUT_SECS,
        max_output,
        "command_help:man_render",
    )?;

    let mut excerpt = if man_render.status_code == 0 {
        normalize_excerpt(&man_render.stdout, max_chars)
    } else {
        String::new()
    };

    let mut packages = Vec::<String>::new();
    let mut package_doc_paths = Vec::<String>::new();

    if excerpt.is_empty() {
        packages = query_owning_packages(&command_path)?;

        if let Some(primary_package) = packages.first() {
            package_doc_paths = list_authoritative_package_docs(primary_package)?;
            if let Some(first_doc) = select_plaintext_doc(&package_doc_paths) {
                if let Ok(content) = fs::read_to_string(first_doc) {
                    excerpt = normalize_excerpt(&content, max_chars);
                }
            }
        }
    }

    if excerpt.is_empty() && man_locations.is_empty() && package_doc_paths.is_empty() {
        return Err(ToolError::not_found(format!(
            "no authoritative man or package documentation found for '{}'; unsafe --help execution is disabled",
            input.command
        )));
    }

    let partial = man_where.stdout_truncated
        || man_where.stderr_truncated
        || man_render.stdout_truncated
        || man_render.stderr_truncated
        || excerpt.chars().count() >= max_chars;

    Ok(ToolResult::new(
        "command_help",
        PrivacyClass::SystemMetadata,
        Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        vec![
            Provenance {
                source: "subprocess".to_string(),
                detail: format!("{} (--where / rendered man page)", man_exe),
            },
            Provenance {
                source: "filesystem".to_string(),
                detail: "optional /usr/share/doc package files".to_string(),
            },
        ],
        partial,
        json!({
            "command": input.command,
            "command_path": command_path,
            "man_locations": man_locations,
            "packages": packages,
            "package_doc_paths": package_doc_paths,
            "excerpt": excerpt,
            "source_policy": "never executes command-selected binaries for help text",
        }),
    ))
}

pub fn command_history(input: CommandHistoryInput) -> Result<ToolResult, ToolError> {
    if input.entries.is_empty() {
        return Err(ToolError::unavailable(
            "history unavailable: only explicitly provided history context is supported",
        ));
    }
    if input.entries.len() > MAX_HISTORY_ENTRIES {
        return Err(ToolError::invalid_input(format!(
            "entries exceeds maximum {}",
            MAX_HISTORY_ENTRIES
        )));
    }

    let limit = bounded_limit(
        input.limit,
        DEFAULT_HISTORY_LIMIT,
        MAX_HISTORY_LIMIT,
        "limit",
    )?;

    let query = input.query.as_ref().map(|value| value.to_lowercase());

    let mut filtered = Vec::new();

    for entry in input.entries {
        if entry.command.trim().is_empty() {
            continue;
        }

        if entry.command.len() > 4_096 {
            continue;
        }

        if input.failures_only && entry.exit_code.unwrap_or(0) == 0 {
            continue;
        }

        if let Some(query) = &query {
            if !entry.command.to_lowercase().contains(query) {
                continue;
            }
        }

        filtered.push(entry);

        if filtered.len() >= limit {
            break;
        }
    }

    Ok(ToolResult::new(
        "command_history",
        PrivacyClass::PotentiallySensitive,
        Duration::from_secs(1),
        vec![Provenance {
            source: "explicit_context".to_string(),
            detail:
                "operates only on caller-supplied history entries; never reads shell history files"
                    .to_string(),
        }],
        filtered.len() >= limit,
        json!({
            "entries": filtered,
            "query": input.query,
            "limit": limit,
            "failures_only": input.failures_only,
            "history_source": "explicit_input_only",
        }),
    ))
}

fn lookup_command_path(command: &str) -> Option<PathBuf> {
    SAFE_COMMAND_PATHS
        .iter()
        .map(|prefix| Path::new(prefix).join(command))
        .find(|candidate| {
            fs::metadata(candidate)
                .map(|meta| {
                    meta.is_file() && meta.permissions().mode() & 0o111 != 0
                })
                .unwrap_or(false)
        })
}

fn validate_optional_man_section(section: Option<&str>) -> Result<(), ToolError> {
    let Some(section) = section else {
        return Ok(());
    };

    if section.is_empty() || section.len() > 16 {
        return Err(ToolError::invalid_input(
            "man section must be 1..=16 characters",
        ));
    }

    if !section
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-'))
    {
        return Err(ToolError::invalid_input(
            "man section contains unsupported characters",
        ));
    }

    Ok(())
}

fn build_man_args(mode: &str, section: Option<&str>, command: &str) -> Vec<String> {
    let mut args = vec![mode.to_string()];
    if let Some(section) = section {
        args.push(section.to_string());
    }
    args.push(command.to_string());
    args
}

fn query_owning_packages(command_path: &Path) -> Result<Vec<String>, ToolError> {
    let dpkg_query = first_available_executable(DPKG_QUERY_CANDIDATES)
        .ok_or_else(|| ToolError::unavailable("dpkg-query not available"))?;

    let args = vec![
        "-S".to_string(),
        "--".to_string(),
        command_path.to_string_lossy().to_string(),
    ];

    let output = run_command(
        dpkg_query,
        &args,
        DEFAULT_TIMEOUT_SECS,
        DEFAULT_OUTPUT_LIMIT,
        "command_help:dpkg_query",
    )?;

    if output.status_code != 0 {
        return Ok(Vec::new());
    }

    let mut packages = Vec::<String>::new();
    for line in output.stdout.lines() {
        if let Some((pkg_segment, _)) = line.split_once(": ") {
            for package in pkg_segment.split(',') {
                let trimmed = package.trim();
                if !trimmed.is_empty() {
                    packages.push(trimmed.to_string());
                }
            }
        }
    }

    packages.sort();
    packages.dedup();
    Ok(packages)
}

fn list_authoritative_package_docs(package: &str) -> Result<Vec<String>, ToolError> {
    let package_base = package.split(':').next().unwrap_or(package);
    let doc_root = Path::new("/usr/share/doc").join(package_base);

    let mut paths = Vec::new();

    let entries = match fs::read_dir(&doc_root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(paths),
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            return Err(ToolError::permission_denied(format!(
                "cannot read package docs at {}: {err}",
                doc_root.display()
            )));
        }
        Err(err) => {
            return Err(ToolError::io(
                format!("reading {}", doc_root.display()),
                err.to_string(),
            ));
        }
    };

    for entry in entries {
        let entry = entry
            .map_err(|err| ToolError::io("iterating package doc dir", err.to_string()))?;
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|value| value.to_str()) {
            let lower = name.to_lowercase();
            if lower.starts_with("readme")
                || lower.starts_with("changelog")
                || lower.starts_with("copyright")
                || lower.contains("debian")
            {
                paths.push(path.to_string_lossy().to_string());
            }
        }
    }

    paths.sort();
    Ok(paths)
}

fn select_plaintext_doc(paths: &[String]) -> Option<&Path> {
    paths
        .iter()
        .map(Path::new)
        .find(|path| !path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| ext == "gz"))
}

fn normalize_excerpt(raw: &str, max_chars: usize) -> String {
    raw.chars().take(max_chars).collect::<String>().trim().to_string()
}
