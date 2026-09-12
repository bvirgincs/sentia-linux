use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::ToolError;

#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub status_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

pub fn first_available_executable(candidates: &[&'static str]) -> Option<&'static str> {
    candidates.iter().copied().find(|candidate| {
        fs::metadata(candidate)
            .map(|meta| {
                let mode = meta.permissions().mode();
                meta.is_file() && mode & 0o111 != 0
            })
            .unwrap_or(false)
    })
}

pub fn run_command(
    executable: &str,
    args: &[String],
    timeout_secs: u64,
    output_limit: usize,
    operation: &str,
) -> Result<CommandOutput, ToolError> {
    if !executable.starts_with('/') {
        return Err(ToolError::invalid_input(
            "executable path must be absolute",
        ));
    }

    let metadata = fs::metadata(executable).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            ToolError::unavailable(format!("required executable unavailable: {executable}"))
        } else {
            ToolError::io(format!("checking {executable}"), err.to_string())
        }
    })?;

    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err(ToolError::unavailable(format!(
            "required executable is not runnable: {executable}"
        )));
    }

    let mut command = Command::new(executable);
    command.args(args);
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.env_clear();
    command.env(
        "PATH",
        "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
    );
    command.env("LANG", "C");
    command.env("LC_ALL", "C");
    command.env("HOME", "/nonexistent");
    command.env("PAGER", "cat");
    command.env("MANPAGER", "cat");
    command.env("SYSTEMD_PAGER", "cat");
    command.env("SYSTEMD_COLORS", "0");

    let mut child = command
        .spawn()
        .map_err(|err| ToolError::io(format!("spawning {operation}"), err.to_string()))?;

    let stdout = child.stdout.take().ok_or_else(|| {
        ToolError::io(
            format!("capturing stdout for {operation}"),
            "missing stdout pipe".to_string(),
        )
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        ToolError::io(
            format!("capturing stderr for {operation}"),
            "missing stderr pipe".to_string(),
        )
    })?;

    let stdout_reader = thread::spawn(move || read_stream_with_limit(stdout, output_limit));
    let stderr_reader = thread::spawn(move || read_stream_with_limit(stderr, output_limit));

    let timeout = Duration::from_secs(timeout_secs);
    let started = Instant::now();

    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|err| ToolError::io(format!("waiting for {operation}"), err.to_string()))?
        {
            let stdout_result = stdout_reader.join().map_err(|_| {
                ToolError::io(
                    format!("joining stdout thread for {operation}"),
                    "reader thread panicked".to_string(),
                )
            })?;
            let stderr_result = stderr_reader.join().map_err(|_| {
                ToolError::io(
                    format!("joining stderr thread for {operation}"),
                    "reader thread panicked".to_string(),
                )
            })?;

            let (stdout_bytes, stdout_truncated) = stdout_result
                .map_err(|err| ToolError::io(format!("reading stdout for {operation}"), err.to_string()))?;
            let (stderr_bytes, stderr_truncated) = stderr_result
                .map_err(|err| ToolError::io(format!("reading stderr for {operation}"), err.to_string()))?;

            return Ok(CommandOutput {
                status_code: status.code().unwrap_or(-1),
                stdout: String::from_utf8_lossy(&stdout_bytes).to_string(),
                stderr: String::from_utf8_lossy(&stderr_bytes).to_string(),
                stdout_truncated,
                stderr_truncated,
            });
        }

        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();

            let _ = stdout_reader.join();
            let _ = stderr_reader.join();

            return Err(ToolError::timeout(operation.to_string(), timeout_secs));
        }

        thread::sleep(Duration::from_millis(20));
    }
}

fn read_stream_with_limit<R: Read>(mut reader: R, limit: usize) -> std::io::Result<(Vec<u8>, bool)> {
    let mut output = Vec::with_capacity(limit.min(8 * 1024));
    let mut buffer = [0_u8; 4096];
    let mut truncated = false;

    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }

        let remaining = limit.saturating_sub(output.len());
        if remaining == 0 {
            truncated = true;
        } else if read <= remaining {
            output.extend_from_slice(&buffer[..read]);
        } else {
            output.extend_from_slice(&buffer[..remaining]);
            truncated = true;
        }
    }

    Ok((output, truncated))
}
