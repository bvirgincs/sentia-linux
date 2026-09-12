use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use crate::monitor::{HealthMonitor, MonitorConfig};
use crate::schema::HealthSnapshot;

#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub socket_path: PathBuf,
    pub monitor: MonitorConfig,
    pub poll_sleep: Duration,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            socket_path: PathBuf::from("/run/sentia/health.sock"),
            monitor: MonitorConfig::default(),
            poll_sleep: Duration::from_millis(200),
        }
    }
}

pub fn run_daemon(config: DaemonConfig) -> io::Result<()> {
    let parent = config
        .socket_path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "socket path has no parent"))?;

    fs::create_dir_all(parent)?;

    if config.socket_path.exists() {
        fs::remove_file(&config.socket_path)?;
    }

    let listener = UnixListener::bind(&config.socket_path)?;
    fs::set_permissions(&config.socket_path, fs::Permissions::from_mode(0o660))?;
    listener.set_nonblocking(true)?;

    let mut monitor = HealthMonitor::new(config.monitor);
    let mut latest = monitor.sample();
    let mut next_sample = Instant::now() + monitor.next_interval();

    loop {
        loop {
            match listener.accept() {
                Ok((mut stream, _addr)) => {
                    if let Err(err) = write_snapshot(&mut stream, &latest) {
                        let _ = writeln!(io::stderr(), "sentia-health: socket write failed: {err}");
                    }
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                    break;
                }
                Err(err) => {
                    return Err(err);
                }
            }
        }

        if Instant::now() >= next_sample {
            latest = monitor.sample();
            next_sample = Instant::now() + monitor.next_interval();
        }

        thread::sleep(config.poll_sleep);
    }
}

pub fn fetch_snapshot_from_socket(path: &Path) -> io::Result<HealthSnapshot> {
    let mut stream = UnixStream::connect(path)?;
    let mut body = String::new();
    stream.read_to_string(&mut body)?;

    let snapshot = serde_json::from_str::<HealthSnapshot>(body.trim()).map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("failed to parse daemon snapshot json: {err}"),
        )
    })?;

    Ok(snapshot)
}

fn write_snapshot(stream: &mut UnixStream, snapshot: &HealthSnapshot) -> io::Result<()> {
    let payload = serde_json::to_vec(snapshot).map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("failed to serialize snapshot: {err}"),
        )
    })?;
    stream.write_all(&payload)?;
    stream.write_all(b"\n")?;
    stream.flush()
}
