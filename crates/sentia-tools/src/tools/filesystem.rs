use std::fs;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::result::{PrivacyClass, Provenance, ToolResult};
use crate::runner::{first_available_executable, run_command};
use crate::validation::{
    bounded_limit, enforce_path_policy, DEFAULT_DIRECTORY_LIMIT, DEFAULT_OUTPUT_LIMIT,
    DEFAULT_TIMEOUT_SECS, MAX_DIRECTORY_LIMIT,
};
use crate::ToolError;

const DF_CANDIDATES: &[&str] = &["/bin/df", "/usr/bin/df"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FilePathInput {
    pub path: PathBuf,
    #[serde(default)]
    pub allow_sensitive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirectoryContentsInput {
    pub path: PathBuf,
    pub limit: Option<usize>,
    #[serde(default)]
    pub include_hidden: bool,
    #[serde(default)]
    pub allow_sensitive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiskUsageInput {
    pub path: PathBuf,
    #[serde(default)]
    pub allow_sensitive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirectoryEntrySummary {
    pub name: String,
    pub path: PathBuf,
    pub file_type: String,
    pub mode_octal: String,
    pub size_bytes: Option<u64>,
    pub symlink_target: Option<PathBuf>,
}

pub fn file_exists(input: FilePathInput) -> Result<ToolResult, ToolError> {
    enforce_path_policy(&input.path, input.allow_sensitive)?;

    let exists = input.path.try_exists().map_err(|err| {
        if err.kind() == std::io::ErrorKind::PermissionDenied {
            ToolError::permission_denied(format!(
                "permission denied checking existence of {}",
                input.path.display()
            ))
        } else {
            ToolError::io(
                format!("checking existence of {}", input.path.display()),
                err.to_string(),
            )
        }
    })?;

    Ok(ToolResult::new(
        "file_exists",
        PrivacyClass::PotentiallySensitive,
        Duration::from_secs(1),
        vec![Provenance {
            source: "filesystem".to_string(),
            detail: "std::fs::try_exists".to_string(),
        }],
        false,
        json!({
            "path": input.path,
            "exists": exists,
        }),
    ))
}

pub fn file_type(input: FilePathInput) -> Result<ToolResult, ToolError> {
    enforce_path_policy(&input.path, input.allow_sensitive)?;

    let file_type_name = match fs::symlink_metadata(&input.path) {
        Ok(metadata) => classify_file_type(metadata.file_type()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => "missing".to_string(),
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            return Err(ToolError::permission_denied(format!(
                "permission denied reading metadata for {}",
                input.path.display()
            )));
        }
        Err(err) => {
            return Err(ToolError::io(
                format!("reading metadata for {}", input.path.display()),
                err.to_string(),
            ));
        }
    };

    Ok(ToolResult::new(
        "file_type",
        PrivacyClass::PotentiallySensitive,
        Duration::from_secs(1),
        vec![Provenance {
            source: "filesystem".to_string(),
            detail: "std::fs::symlink_metadata".to_string(),
        }],
        false,
        json!({
            "path": input.path,
            "file_type": file_type_name,
        }),
    ))
}

pub fn file_permissions(input: FilePathInput) -> Result<ToolResult, ToolError> {
    enforce_path_policy(&input.path, input.allow_sensitive)?;

    let metadata = fs::symlink_metadata(&input.path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            ToolError::not_found(format!("{} does not exist", input.path.display()))
        } else if err.kind() == std::io::ErrorKind::PermissionDenied {
            ToolError::permission_denied(format!(
                "permission denied reading metadata for {}",
                input.path.display()
            ))
        } else {
            ToolError::io(
                format!("reading metadata for {}", input.path.display()),
                err.to_string(),
            )
        }
    })?;

    let mode = metadata.permissions().mode() & 0o7777;

    Ok(ToolResult::new(
        "file_permissions",
        PrivacyClass::PotentiallySensitive,
        Duration::from_secs(1),
        vec![Provenance {
            source: "filesystem".to_string(),
            detail: "std::fs::symlink_metadata + unix mode bits".to_string(),
        }],
        false,
        json!({
            "path": input.path,
            "mode": mode,
            "mode_octal": format!("{:04o}", mode),
            "uid": metadata.uid(),
            "gid": metadata.gid(),
            "readonly": metadata.permissions().readonly(),
        }),
    ))
}

pub fn directory_contents(input: DirectoryContentsInput) -> Result<ToolResult, ToolError> {
    enforce_path_policy(&input.path, input.allow_sensitive)?;

    let limit = bounded_limit(
        input.limit,
        DEFAULT_DIRECTORY_LIMIT,
        MAX_DIRECTORY_LIMIT,
        "limit",
    )?;

    let meta = fs::symlink_metadata(&input.path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            ToolError::not_found(format!("{} does not exist", input.path.display()))
        } else if err.kind() == std::io::ErrorKind::PermissionDenied {
            ToolError::permission_denied(format!(
                "permission denied reading {}",
                input.path.display()
            ))
        } else {
            ToolError::io(format!("reading {}", input.path.display()), err.to_string())
        }
    })?;

    if meta.file_type().is_symlink() {
        return Err(ToolError::invalid_input(
            "directory path must not be a symlink target to reduce race risk",
        ));
    }

    if !meta.is_dir() {
        return Err(ToolError::invalid_input("path is not a directory"));
    }

    let mut entries = Vec::<DirectoryEntrySummary>::new();

    let read_dir = fs::read_dir(&input.path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::PermissionDenied {
            ToolError::permission_denied(format!(
                "permission denied listing directory {}",
                input.path.display()
            ))
        } else {
            ToolError::io(
                format!("listing directory {}", input.path.display()),
                err.to_string(),
            )
        }
    })?;

    for entry in read_dir {
        let entry = entry.map_err(|err| ToolError::io("iterating directory", err.to_string()))?;
        let name = entry.file_name().to_string_lossy().to_string();

        if !input.include_hidden && name.starts_with('.') {
            continue;
        }

        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|err| ToolError::io(format!("reading metadata for {}", path.display()), err.to_string()))?;

        let mode = metadata.permissions().mode() & 0o7777;
        let symlink_target = if metadata.file_type().is_symlink() {
            fs::read_link(&path).ok()
        } else {
            None
        };

        entries.push(DirectoryEntrySummary {
            name,
            path,
            file_type: classify_file_type(metadata.file_type()),
            mode_octal: format!("{:04o}", mode),
            size_bytes: if metadata.is_file() {
                Some(metadata.size())
            } else {
                None
            },
            symlink_target,
        });
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name));

    let partial = entries.len() > limit;
    if entries.len() > limit {
        entries.truncate(limit);
    }

    Ok(ToolResult::new(
        "directory_contents",
        PrivacyClass::PotentiallySensitive,
        Duration::from_secs(2),
        vec![Provenance {
            source: "filesystem".to_string(),
            detail: "std::fs::read_dir + symlink_metadata".to_string(),
        }],
        partial,
        json!({
            "path": input.path,
            "entries": entries,
            "include_hidden": input.include_hidden,
            "limit": limit,
        }),
    ))
}

pub fn disk_usage(input: DiskUsageInput) -> Result<ToolResult, ToolError> {
    enforce_path_policy(&input.path, input.allow_sensitive)?;

    let exists = input.path.try_exists().map_err(|err| {
        if err.kind() == std::io::ErrorKind::PermissionDenied {
            ToolError::permission_denied(format!(
                "permission denied checking {}",
                input.path.display()
            ))
        } else {
            ToolError::io(
                format!("checking {}", input.path.display()),
                err.to_string(),
            )
        }
    })?;

    if !exists {
        return Err(ToolError::not_found(format!(
            "path {} does not exist",
            input.path.display()
        )));
    }

    let df = first_available_executable(DF_CANDIDATES)
        .ok_or_else(|| ToolError::unavailable("df executable not available"))?;

    let args = vec![
        "--block-size=1".to_string(),
        "--output=source,fstype,size,used,avail,pcent,target".to_string(),
        "--".to_string(),
        input.path.to_string_lossy().to_string(),
    ];

    let output = run_command(df, &args, DEFAULT_TIMEOUT_SECS, DEFAULT_OUTPUT_LIMIT, "disk_usage")?;
    if output.status_code != 0 {
        let lower = output.stderr.to_lowercase();
        if lower.contains("permission denied") {
            return Err(ToolError::permission_denied(output.stderr.trim().to_string()));
        }
        return Err(ToolError::unavailable(format!(
            "df failed (exit {}): {}",
            output.status_code,
            output.stderr.trim()
        )));
    }

    let mut lines = output
        .stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());

    let _header = lines
        .next()
        .ok_or_else(|| ToolError::parse("df", "missing output header"))?;
    let data_line = lines
        .next()
        .ok_or_else(|| ToolError::parse("df", "missing output data"))?;

    let fields: Vec<&str> = data_line.split_whitespace().collect();
    if fields.len() < 7 {
        return Err(ToolError::parse(
            "df",
            format!("expected >=7 columns, got {}", fields.len()),
        ));
    }

    let source = fields[0].to_string();
    let fstype = fields[1].to_string();
    let size_bytes = parse_u64_field(fields[2], "size")?;
    let used_bytes = parse_u64_field(fields[3], "used")?;
    let avail_bytes = parse_u64_field(fields[4], "avail")?;
    let used_percent = fields[5]
        .trim_end_matches('%')
        .parse::<u8>()
        .map_err(|err| ToolError::parse("df pcent", err.to_string()))?;
    let mount_point = fields[6..].join(" ");

    Ok(ToolResult::new(
        "disk_usage",
        PrivacyClass::SystemMetadata,
        Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        vec![Provenance {
            source: "subprocess".to_string(),
            detail: format!("{} --output=source,fstype,size,used,avail,pcent,target", df),
        }],
        output.stdout_truncated || output.stderr_truncated,
        json!({
            "path": input.path,
            "filesystem": source,
            "filesystem_type": fstype,
            "size_bytes": size_bytes,
            "used_bytes": used_bytes,
            "available_bytes": avail_bytes,
            "used_percent": used_percent,
            "mount_point": mount_point,
        }),
    ))
}

fn classify_file_type(file_type: fs::FileType) -> String {
    if file_type.is_file() {
        "file".to_string()
    } else if file_type.is_dir() {
        "directory".to_string()
    } else if file_type.is_symlink() {
        "symlink".to_string()
    } else if file_type.is_char_device() {
        "char_device".to_string()
    } else if file_type.is_block_device() {
        "block_device".to_string()
    } else if file_type.is_fifo() {
        "fifo".to_string()
    } else if file_type.is_socket() {
        "socket".to_string()
    } else {
        "unknown".to_string()
    }
}

fn parse_u64_field(value: &str, field: &str) -> Result<u64, ToolError> {
    value
        .parse::<u64>()
        .map_err(|err| ToolError::parse(format!("df {field}"), err.to_string()))
}
