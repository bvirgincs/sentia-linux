use std::fs::File;
use std::io::Read;
use std::path::{Component, Path};

use crate::ToolError;

pub const DEFAULT_TIMEOUT_SECS: u64 = 4;
pub const DEFAULT_OUTPUT_LIMIT: usize = 64 * 1024;
pub const DEFAULT_HISTORY_LIMIT: usize = 100;
pub const MAX_HISTORY_ENTRIES: usize = 2_000;
pub const MAX_HISTORY_LIMIT: usize = 500;
pub const DEFAULT_DIRECTORY_LIMIT: usize = 200;
pub const MAX_DIRECTORY_LIMIT: usize = 2_000;
pub const DEFAULT_LIST_LIMIT: usize = 200;
pub const MAX_LIST_LIMIT: usize = 2_000;
pub const DEFAULT_LOG_LINES: usize = 120;
pub const MAX_LOG_LINES: usize = 1_000;
pub const MAX_COMMAND_TOKEN_LEN: usize = 80;
pub const MAX_SERVICE_NAME_LEN: usize = 128;

pub fn validate_command_token(token: &str) -> Result<(), ToolError> {
    if token.is_empty() {
        return Err(ToolError::invalid_input("command token cannot be empty"));
    }
    if token.len() > MAX_COMMAND_TOKEN_LEN {
        return Err(ToolError::invalid_input(format!(
            "command token exceeds {MAX_COMMAND_TOKEN_LEN} characters"
        )));
    }
    if token.contains('/') {
        return Err(ToolError::invalid_input(
            "command token must not include path separators",
        ));
    }
    if !token
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '+'))
    {
        return Err(ToolError::invalid_input(
            "command token contains unsupported characters",
        ));
    }
    Ok(())
}

pub fn normalize_service_name(service: &str) -> Result<String, ToolError> {
    if service.is_empty() {
        return Err(ToolError::invalid_input("service name cannot be empty"));
    }
    if service.len() > MAX_SERVICE_NAME_LEN {
        return Err(ToolError::invalid_input(format!(
            "service name exceeds {MAX_SERVICE_NAME_LEN} characters"
        )));
    }
    if !service
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '@'))
    {
        return Err(ToolError::invalid_input(
            "service name contains unsupported characters",
        ));
    }

    if service.ends_with(".service") {
        Ok(service.to_string())
    } else {
        Ok(format!("{service}.service"))
    }
}

pub fn bounded_limit(
    user_limit: Option<usize>,
    default_value: usize,
    max_value: usize,
    field_name: &str,
) -> Result<usize, ToolError> {
    let value = user_limit.unwrap_or(default_value);
    if value == 0 {
        return Err(ToolError::invalid_input(format!(
            "{field_name} must be greater than zero"
        )));
    }
    if value > max_value {
        return Err(ToolError::invalid_input(format!(
            "{field_name} must be <= {max_value}"
        )));
    }
    Ok(value)
}

pub fn validate_absolute_path(path: &Path) -> Result<(), ToolError> {
    if !path.is_absolute() {
        return Err(ToolError::invalid_input(
            "path must be an absolute filesystem path",
        ));
    }

    for component in path.components() {
        if matches!(component, Component::ParentDir | Component::CurDir) {
            return Err(ToolError::invalid_input(
                "path must not contain dot or dot-dot components",
            ));
        }
    }

    Ok(())
}

pub fn enforce_path_policy(path: &Path, allow_sensitive: bool) -> Result<(), ToolError> {
    validate_absolute_path(path)?;

    if allow_sensitive {
        return Ok(());
    }

    let path_str = path.to_string_lossy();

    let denied_reason = if path_str.starts_with("/root") {
        Some("root-owned home content")
    } else if matches!(
        path_str.as_ref(),
        "/etc/shadow" | "/etc/gshadow" | "/etc/sudoers" | "/etc/security/opasswd"
    ) {
        Some("credential or password policy content")
    } else if path_str.contains("/.ssh/") || path_str.ends_with("/.ssh") {
        Some("ssh credential material")
    } else if path_str.contains("/.gnupg/") || path_str.ends_with("/.gnupg") {
        Some("gnupg private material")
    } else if path_str.starts_with("/proc/")
        && ["environ", "mem", "kcore", "cmdline"].iter().any(|name| {
            path.file_name()
                .and_then(|v| v.to_str())
                .is_some_and(|value| value == *name)
        })
    {
        Some("process-sensitive kernel pseudo files")
    } else if path_str.ends_with(".pem")
        || path_str.ends_with(".key")
        || path_str.ends_with(".p12")
    {
        Some("likely key or certificate material")
    } else {
        None
    };

    if let Some(reason) = denied_reason {
        return Err(ToolError::permission_denied(format!(
            "access to sensitive path denied by default policy ({reason})"
        )));
    }

    Ok(())
}

pub fn read_text_file_bounded(path: &Path, limit_bytes: usize) -> Result<String, ToolError> {
    let file = File::open(path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::PermissionDenied {
            ToolError::permission_denied(format!("cannot read {}: {err}", path.display()))
        } else {
            ToolError::io(
                format!("opening {}", path.display()),
                err.to_string(),
            )
        }
    })?;

    let mut buffer = Vec::new();
    file.take((limit_bytes + 1) as u64)
        .read_to_end(&mut buffer)
        .map_err(|err| ToolError::io(format!("reading {}", path.display()), err.to_string()))?;

    if buffer.len() > limit_bytes {
        return Err(ToolError::output_limit_exceeded(
            path.display().to_string(),
            limit_bytes,
        ));
    }

    Ok(String::from_utf8_lossy(&buffer).to_string())
}
