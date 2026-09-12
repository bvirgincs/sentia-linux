use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::result::{PrivacyClass, Provenance, ToolResult};
use crate::runner::{first_available_executable, run_command};
use crate::validation::{enforce_path_policy, DEFAULT_OUTPUT_LIMIT, DEFAULT_TIMEOUT_SECS};
use crate::ToolError;

const DPKG_QUERY_CANDIDATES: &[&str] = &["/usr/bin/dpkg-query", "/bin/dpkg-query"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackageOwnsFileInput {
    pub path: PathBuf,
    #[serde(default)]
    pub allow_sensitive: bool,
}

pub fn package_owns_file(input: PackageOwnsFileInput) -> Result<ToolResult, ToolError> {
    enforce_path_policy(&input.path, input.allow_sensitive)?;

    let dpkg_query = first_available_executable(DPKG_QUERY_CANDIDATES)
        .ok_or_else(|| ToolError::unavailable("dpkg-query not available on this host"))?;

    let args = vec![
        "-S".to_string(),
        "--".to_string(),
        input.path.to_string_lossy().to_string(),
    ];

    let output = run_command(
        dpkg_query,
        &args,
        DEFAULT_TIMEOUT_SECS,
        DEFAULT_OUTPUT_LIMIT,
        "package_owns_file",
    )?;

    if output.status_code != 0 {
        let stderr = output.stderr.to_lowercase();
        if stderr.contains("no path found matching pattern") {
            return Err(ToolError::not_found(format!(
                "no installed package owns {}",
                input.path.display()
            )));
        }
        if stderr.contains("permission denied") {
            return Err(ToolError::permission_denied(output.stderr.trim().to_string()));
        }
        return Err(ToolError::unavailable(format!(
            "dpkg-query failed (exit {}): {}",
            output.status_code,
            output.stderr.trim()
        )));
    }

    let mut owners = Vec::new();
    for line in output.stdout.lines() {
        if let Some((packages_part, _path_part)) = line.split_once(": ") {
            for package in packages_part.split(',') {
                let trimmed = package.trim();
                if !trimmed.is_empty() {
                    owners.push(trimmed.to_string());
                }
            }
        }
    }

    owners.sort();
    owners.dedup();

    if owners.is_empty() {
        return Err(ToolError::parse(
            "dpkg-query -S",
            "unable to parse package ownership output",
        ));
    }

    let partial = output.stdout_truncated || output.stderr_truncated;

    Ok(ToolResult::new(
        "package_owns_file",
        PrivacyClass::SystemMetadata,
        Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        vec![Provenance {
            source: "subprocess".to_string(),
            detail: format!("{} -S -- <path>", dpkg_query),
        }],
        partial,
        json!({
            "path": input.path,
            "owners": owners,
        }),
    ))
}
