// SPDX-License-Identifier: Apache-2.0
use crate::{broker::CanonicalPlan, digest, Error, Operation, Result, ServiceAction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Component, Path};
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

pub const SYSTEMCTL: &str = "/usr/bin/systemctl";
pub const APT_WORKER: &str = "/usr/libexec/sentia/sentia-apt-worker";
const MAX_OUTPUT: usize = 1_048_576;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

pub fn service_arguments(action: &ServiceAction, unit: &str) -> Result<Vec<String>> {
    crate::validate_unit(unit)?;
    Ok(vec![
        "--no-ask-password".into(),
        "--no-pager".into(),
        action.argument().into(),
        "--".into(),
        unit.into(),
    ])
}

pub fn trusted_path(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        return Err(Error("untrusted_path"));
    }
    let mut current = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir | Component::Normal(_) => current.push(component),
            _ => return Err(Error("untrusted_path")),
        }
        let metadata =
            std::fs::symlink_metadata(&current).map_err(|_| Error("path_unavailable"))?;
        if metadata.file_type().is_symlink()
            || metadata.uid() != 0
            || metadata.mode() & 0o022 != 0
        {
            return Err(Error("untrusted_path"));
        }
    }
    Ok(())
}

fn trusted_file_digest(path: &str) -> Result<String> {
    let original = Path::new(path);
    // systemd legitimately resolves administrator-created unit symlinks.
    // Validate every link's parent before resolving, then the canonical chain.
    trusted_path(original.parent().ok_or(Error("untrusted_path"))?)?;
    let link_metadata =
        std::fs::symlink_metadata(original).map_err(|_| Error("path_unavailable"))?;
    if link_metadata.uid() != 0 {
        return Err(Error("untrusted_path"));
    }
    let canonical = original
        .canonicalize()
        .map_err(|_| Error("path_unavailable"))?;
    if ![
        Path::new("/etc/systemd/system"),
        Path::new("/usr/lib/systemd/system"),
        Path::new("/run/systemd"),
    ]
    .iter()
    .any(|root| canonical.starts_with(root))
    {
        return Err(Error("untrusted_unit_file"));
    }
    trusted_path(&canonical)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(canonical)
        .map_err(|_| Error("unit_file_unavailable"))?;
    let metadata = file.metadata().map_err(|_| Error("unit_file_unavailable"))?;
    if !metadata.is_file() || metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
        return Err(Error("untrusted_unit_file"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_OUTPUT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error("unit_file_unavailable"))?;
    if bytes.len() > MAX_OUTPUT {
        return Err(Error("unit_file_too_large"));
    }
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

async fn bounded_read<R: AsyncRead + Unpin>(reader: R) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_OUTPUT as u64 + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| Error("subprocess_io_failed"))?;
    if bytes.len() > MAX_OUTPUT {
        return Err(Error("subprocess_output_limit"));
    }
    Ok(bytes)
}

pub async fn run_fixed(
    executable: &'static str,
    arguments: &[String],
    input: Option<&[u8]>,
    timeout: Duration,
) -> Result<Vec<u8>> {
    let output = run_helper(executable, arguments, input, timeout).await?;
    if output.success {
        Ok(output.stdout)
    } else {
        Err(Error("subprocess_failed"))
    }
}

struct HelperOutput {
    success: bool,
    stdout: Vec<u8>,
}

async fn run_helper(
    executable: &'static str,
    arguments: &[String],
    input: Option<&[u8]>,
    timeout: Duration,
) -> Result<HelperOutput> {
    if executable != SYSTEMCTL && executable != APT_WORKER {
        return Err(Error("executable_not_allowed"));
    }
    trusted_path(Path::new(executable))?;
    let metadata = std::fs::metadata(executable).map_err(|_| Error("helper_unavailable"))?;
    if !metadata.is_file() || metadata.mode() & 0o111 == 0 {
        return Err(Error("helper_unavailable"));
    }
    let mut command = Command::new(executable);
    command
        .args(arguments)
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin")
        .env("LANG", "C.UTF-8")
        .env("LC_ALL", "C.UTF-8")
        .env("HOME", "/")
        .env("SYSTEMD_PAGER", "")
        .env("SYSTEMD_COLORS", "0")
        .env("DEBIAN_FRONTEND", "noninteractive")
        .current_dir("/")
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // A new process group makes cancellation cover package-manager descendants
    // without ever signalling unrelated processes by executable name.
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().map_err(|_| Error("helper_unavailable"))?;
    let pid = child.id().ok_or(Error("helper_unavailable"))? as i32;
    let stdout = child.stdout.take().ok_or(Error("subprocess_io_failed"))?;
    let stderr = child.stderr.take().ok_or(Error("subprocess_io_failed"))?;
    let mut stdin = child.stdin.take();
    let execution = async {
        let write = async {
            if let (Some(stdin), Some(input)) = (stdin.as_mut(), input) {
                stdin
                    .write_all(input)
                    .await
                    .map_err(|_| Error("subprocess_io_failed"))?;
                stdin
                    .shutdown()
                    .await
                    .map_err(|_| Error("subprocess_io_failed"))?;
            }
            drop(stdin.take());
            Ok(())
        };
        let (_, output, _error) =
            tokio::try_join!(write, bounded_read(stdout), bounded_read(stderr))?;
        let status = child.wait().await.map_err(|_| Error("subprocess_failed"))?;
        Ok((status, output))
    };
    match tokio::time::timeout(timeout, execution).await {
        Ok(Ok((status, output))) => Ok(HelperOutput {
            success: status.success(),
            stdout: output,
        }),
        result => {
            unsafe {
                libc::kill(-pid, libc::SIGTERM);
            }
            // Escalation is only cleanup of our own helper process group,
            // never part of the public process-termination operation.
            tokio::time::sleep(Duration::from_millis(500)).await;
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
            let _ = child.wait().await;
            match result {
                Ok(Err(error)) => Err(error),
                _ => Err(Error("subprocess_timeout")),
            }
        }
    }
}

pub async fn service_state(unit: &str) -> Result<Value> {
    crate::validate_unit(unit)?;
    let output = run_fixed(
        SYSTEMCTL,
        &[
            "--no-pager".into(),
            "show".into(),
            "--property=Id,LoadState,ActiveState,SubState,UnitFileState,FragmentPath,DropInPaths,NeedDaemonReload".into(),
            "--".into(),
            unit.into(),
        ],
        None,
        COMMAND_TIMEOUT,
    )
    .await?;
    let text = String::from_utf8(output).map_err(|_| Error("invalid_helper_output"))?;
    let mut properties = BTreeMap::new();
    for line in text.lines() {
        let (key, value) = line.split_once('=').ok_or(Error("invalid_helper_output"))?;
        properties.insert(key.to_owned(), value.to_owned());
    }
    if properties.get("LoadState").map(String::as_str) != Some("loaded")
        || properties.get("NeedDaemonReload").map(String::as_str) != Some("no")
    {
        return Err(Error("unit_unavailable_or_stale"));
    }
    let fragment = properties
        .get("FragmentPath")
        .filter(|p| !p.is_empty())
        .ok_or(Error("transient_units_not_allowed"))?;
    let mut files = BTreeMap::new();
    files.insert(fragment.clone(), trusted_file_digest(fragment)?);
    if let Some(dropins) = properties.get("DropInPaths") {
        for path in dropins.split_whitespace() {
            files.insert(path.to_owned(), trusted_file_digest(path)?);
        }
    }
    Ok(json!({"properties":properties, "files":files}))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessState {
    pub pid: u32,
    pub start_ticks: u64,
    pub uid: u32,
    pub name: String,
}

pub fn parse_start_ticks(stat: &str) -> Result<u64> {
    let (_, fields) = stat.rsplit_once(')').ok_or(Error("invalid_process_state"))?;
    fields
        .split_whitespace()
        .nth(19)
        .ok_or(Error("invalid_process_state"))?
        .parse()
        .map_err(|_| Error("invalid_process_state"))
}

pub fn process_start(pid: u32) -> Result<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .map_err(|_| Error("process_unavailable"))?;
    parse_start_ticks(&stat)
}

pub fn process_state(pid: u32) -> Result<ProcessState> {
    Operation::ProcessKill { pid }.validate()?;
    if pid == std::process::id() {
        return Err(Error("cannot_terminate_broker"));
    }
    let start_ticks = process_start(pid)?;
    let metadata = std::fs::metadata(format!("/proc/{pid}"))
        .map_err(|_| Error("process_unavailable"))?;
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .map_err(|_| Error("process_unavailable"))?;
    let (_, rest) = stat.split_once('(').ok_or(Error("invalid_process_state"))?;
    let (name, _) = rest.rsplit_once(')').ok_or(Error("invalid_process_state"))?;
    if start_ticks != process_start(pid)? {
        return Err(Error("process_changed"));
    }
    Ok(ProcessState {
        pid,
        start_ticks,
        uid: metadata.uid(),
        name: name.into(),
    })
}

pub struct ProcessHandle(OwnedFd);

impl ProcessHandle {
    pub fn open(pid: u32) -> Result<Self> {
        Operation::ProcessKill { pid }.validate()?;
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        if fd < 0 {
            return Err(Error("pidfd_unavailable"));
        }
        Ok(Self(unsafe { OwnedFd::from_raw_fd(fd as i32) }))
    }

    pub fn exited(&self) -> Result<bool> {
        let mut poll = libc::pollfd {
            fd: self.0.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        if unsafe { libc::poll(&mut poll, 1, 0) } < 0 {
            return Err(Error("process_check_failed"));
        }
        Ok(poll.revents != 0)
    }

    pub fn terminate(&self) -> Result<()> {
        let result = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                self.0.as_raw_fd(),
                libc::SIGTERM,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
        if result < 0 {
            return Err(Error("process_signal_denied"));
        }
        Ok(())
    }
}

pub async fn snapshot(operation: &Operation) -> Result<Value> {
    match operation {
        Operation::Service { unit, .. } => service_state(unit).await,
        Operation::ProcessKill { pid } => {
            serde_json::to_value(process_state(*pid)?).map_err(|_| Error("serialization_failed"))
        }
        _ => apt_plan(operation).await,
    }
}

pub async fn execute(plan: &CanonicalPlan) -> Result<Value> {
    let operation = &plan.operation;
    let expected_state = &plan.state;
    operation.validate()?;
    match operation {
        Operation::Service { action, unit } => {
            if service_state(unit).await? != *expected_state {
                return Err(Error("state_changed_prepare_again"));
            }
            ensure_unexpired(plan)?;
            run_fixed(
                SYSTEMCTL,
                &service_arguments(action, unit)?,
                None,
                COMMAND_TIMEOUT,
            )
            .await?;
            Ok(json!({"version":1,"status":"completed","state":service_state(unit).await?}))
        }
        Operation::ProcessKill { pid } => {
            let handle = ProcessHandle::open(*pid)?;
            let current = serde_json::to_value(process_state(*pid)?)
                .map_err(|_| Error("serialization_failed"))?;
            if handle.exited()? || current != *expected_state {
                return Err(Error("process_changed"));
            }
            ensure_unexpired(plan)?;
            handle.terminate()?;
            for _ in 0..30 {
                if handle.exited()? {
                    return Ok(json!({"version":1,"status":"terminated","signal":"SIGTERM"}));
                }

                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Ok(json!({"version":1,"status":"signal_sent_process_still_running","signal":"SIGTERM"}))
        }
        _ => apt_call(operation, Some(plan)).await,
    }
}

fn ensure_unexpired(plan: &CanonicalPlan) -> Result<()> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error("clock_unavailable"))?
        .as_secs();
    if now < plan.issued_at || now >= plan.expires_at {
        return Err(Error("plan_expired"));
    }
    Ok(())
}

// The worker is a separate GPL-compatible executable, not linked into this
// Apache-licensed broker. The JSON contract is documented alongside the service.
async fn apt_plan(operation: &Operation) -> Result<Value> {
    apt_call(operation, None).await
}

fn utc_timestamp(seconds: u64) -> Result<String> {
    let seconds: libc::time_t = seconds.try_into().map_err(|_| Error("invalid_time"))?;
    let mut date = std::mem::MaybeUninit::<libc::tm>::uninit();
    if unsafe { libc::gmtime_r(&seconds, date.as_mut_ptr()) }.is_null() {
        return Err(Error("invalid_time"));
    }
    let date = unsafe { date.assume_init() };
    Ok(format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        date.tm_year + 1900, date.tm_mon + 1, date.tm_mday,
        date.tm_hour, date.tm_min, date.tm_sec))
}

pub fn apt_request(operation: &Operation, expected: Option<&CanonicalPlan>) -> Result<Value> {
    operation.validate()?;
    let (name, packages) = match operation {
        Operation::AptInstall { packages } => ("apt_install", Some(packages)),
        Operation::AptRemove { packages } => ("apt_remove", Some(packages)),
        Operation::AptUpdate => ("apt_update", None),
        Operation::AptUpgrade => ("apt_upgrade", None),
        _ => return Err(Error("invalid_package_operation")),
    };
    let mut arguments = json!({"mode":if expected.is_some() { "execute" } else { "plan" }});
    if let Some(packages) = packages {
        arguments["packages"] = json!(packages);
    }
    let mut request = json!({
        "request_id": expected.map(|p| p.id.as_str()).unwrap_or("sentia-root-plan"),
        "protocol_version":"1.0",
        "operation": name,
        "arguments": arguments,
    });
    if let Some(plan) = expected {
        let worker_digest = plan.state.get("plan_digest").and_then(Value::as_str)
            .ok_or(Error("invalid_worker_plan"))?;
        if plan.operation != *operation || worker_digest.len() != 64
            || !worker_digest.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(Error("invalid_worker_plan"));
        }
        request["approval"] = json!({
            "plan_digest": worker_digest,
            "broker_session_id": digest(&plan.caller)?,
            "authorization_id": plan.id,
            "expires_at": utc_timestamp(plan.expires_at)?,
            "allow_source_change": false,
            "allow_essential_removal": false,
            "allow_held_change": false,
        });
    }
    Ok(request)
}

async fn apt_call(operation: &Operation, expected: Option<&CanonicalPlan>) -> Result<Value> {
    if let Some(plan) = expected {
        ensure_unexpired(plan)?;
    }
    let request = apt_request(operation, expected)?;
    let input = serde_json::to_vec(&request).map_err(|_| Error("serialization_failed"))?;
    let output = run_helper(
        APT_WORKER,
        &[],
        Some(&input),
        if expected.is_some() {
            Duration::from_secs(900)
        } else {
            COMMAND_TIMEOUT
        },
    )
    .await?;
    interpret_apt_response(&output.stdout, output.success, &request, expected.is_some())
}

pub fn interpret_apt_response(
    output: &[u8],
    successful_exit: bool,
    request: &Value,
    executing: bool,
) -> Result<Value> {
    let response: Value =
        serde_json::from_slice(output).map_err(|_| Error("invalid_worker_response"))?;
    if response.get("protocol_version").and_then(Value::as_str) != Some("1.0")
        || response.get("request_id") != request.get("request_id")
        || response.get("operation") != request.get("operation")
    {
        return Err(Error("package_worker_failed"));
    }
    if response.get("status").and_then(Value::as_str) == Some("error") {
        return Err(match response.pointer("/error/code").and_then(Value::as_str) {
            Some("plan_digest_mismatch") => Error("state_changed_prepare_again"),
            _ => Error("package_worker_failed"),
        });
    }
    if !successful_exit
        || response.get("status").and_then(Value::as_str) != Some("ok")
        || !response.get("result").is_some_and(Value::is_object)
    {
        return Err(Error("invalid_worker_response"));
    }
    if executing {
        Ok(response)
    } else {
        let result = response.get("result").ok_or(Error("invalid_worker_plan"))?;
        let plan = result.get("canonical_plan").ok_or(Error("invalid_worker_plan"))?;
        let worker_digest = result.get("plan_digest").and_then(Value::as_str)
            .ok_or(Error("invalid_worker_plan"))?;
        // Bound retained metadata separately from the worker's output cap.
        if serde_json::to_vec(plan).map_err(|_| Error("invalid_worker_plan"))?.len() > 262_144 {
            return Err(Error("package_plan_too_large"));
        }
        if worker_digest.len() != 64
            || !worker_digest.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(Error("invalid_worker_plan"));
        }
        // Preserve the worker's digest: its native canonical JSON encoding can
        // differ from serde's Unicode encoding while describing the same plan.
        Ok(json!({"canonical_plan":plan,"plan_digest":worker_digest}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn helper_output_is_bounded_and_arbitrary_executables_are_rejected() {
        assert_eq!(bounded_read(&b"ok"[..]).await.unwrap(), b"ok");
        let oversized = vec![0_u8; MAX_OUTPUT + 1];
        assert_eq!(
            bounded_read(&oversized[..]).await.unwrap_err().0,
            "subprocess_output_limit"
        );
        assert_eq!(
            run_fixed("/bin/sh", &[], None, Duration::from_secs(1))
                .await.unwrap_err().0,
            "executable_not_allowed"
        );
    }
}
