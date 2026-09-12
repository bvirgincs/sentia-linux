use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;
use std::{io, io::Write};

use sentia_health::{
    fetch_snapshot_from_socket, run_daemon, AdaptiveIntervals, DaemonConfig, HealthMonitor,
    MonitorConfig,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Daemon,
    Json,
}

#[derive(Debug, Clone)]
struct Cli {
    mode: Mode,
    from_socket: bool,
    socket_path: PathBuf,
    min_interval_ms: u64,
    normal_interval_ms: u64,
    max_interval_ms: u64,
    max_kernel_errors: usize,
}

impl Default for Cli {
    fn default() -> Self {
        Self {
            mode: Mode::Daemon,
            from_socket: false,
            socket_path: PathBuf::from("/run/sentia/health.sock"),
            min_interval_ms: 2_000,
            normal_interval_ms: 5_000,
            max_interval_ms: 20_000,
            max_kernel_errors: 16,
        }
    }
}

fn main() -> ExitCode {
    let cli = match parse_cli(std::env::args().skip(1).collect()) {
        Ok(cli) => cli,
        Err(err) => {
            eprintln!("{err}");
            print_usage();
            return ExitCode::from(2);
        }
    };

    let monitor_config = MonitorConfig {
        collector: sentia_health::CollectorConfig {
            max_kernel_errors: cli.max_kernel_errors,
            ..Default::default()
        },
        intervals: AdaptiveIntervals {
            min: Duration::from_millis(cli.min_interval_ms.max(250)),
            normal: Duration::from_millis(cli.normal_interval_ms.max(cli.min_interval_ms)),
            max: Duration::from_millis(cli.max_interval_ms.max(cli.normal_interval_ms)),
        },
        ..Default::default()
    };

    match cli.mode {
        Mode::Daemon => {
            let daemon = DaemonConfig {
                socket_path: cli.socket_path,
                monitor: monitor_config,
                ..Default::default()
            };
            if let Err(err) = run_daemon(daemon) {
                eprintln!("sentia-health daemon failed: {err}");
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
        Mode::Json => {
            let result = if cli.from_socket {
                fetch_snapshot_from_socket(&cli.socket_path)
            } else {
                let mut monitor = HealthMonitor::new(monitor_config);
                Ok(monitor.sample())
            };

            match result {
                Ok(snapshot) => match serde_json::to_string_pretty(&snapshot) {
                    Ok(text) => {
                        match write_stdout_line(&text) {
                            Ok(()) => ExitCode::SUCCESS,
                            Err(err) if err.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
                            Err(err) => {
                                eprintln!("failed to write snapshot: {err}");
                                ExitCode::FAILURE
                            }
                        }
                    }
                    Err(err) => {
                        eprintln!("failed to serialize snapshot: {err}");
                        ExitCode::FAILURE
                    }
                },
                Err(err) => {
                    eprintln!("failed to fetch snapshot: {err}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}

fn parse_cli(args: Vec<String>) -> Result<Cli, String> {
    let mut cli = Cli::default();

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => cli.mode = Mode::Json,
            "--daemon" => cli.mode = Mode::Daemon,
            "--from-socket" => cli.from_socket = true,
            "--socket-path" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err("--socket-path expects a path".to_string());
                };
                cli.socket_path = PathBuf::from(value);
            }
            "--min-interval-ms" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err("--min-interval-ms expects a value".to_string());
                };
                cli.min_interval_ms = value
                    .parse::<u64>()
                    .map_err(|_| "--min-interval-ms must be an integer".to_string())?;
            }
            "--interval-ms" | "--normal-interval-ms" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err("--interval-ms expects a value".to_string());
                };
                cli.normal_interval_ms = value
                    .parse::<u64>()
                    .map_err(|_| "--interval-ms must be an integer".to_string())?;
            }
            "--max-interval-ms" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err("--max-interval-ms expects a value".to_string());
                };
                cli.max_interval_ms = value
                    .parse::<u64>()
                    .map_err(|_| "--max-interval-ms must be an integer".to_string())?;
            }
            "--max-kernel-errors" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err("--max-kernel-errors expects a value".to_string());
                };
                cli.max_kernel_errors = value
                    .parse::<usize>()
                    .map_err(|_| "--max-kernel-errors must be an integer".to_string())?;
            }
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            unknown => {
                return Err(format!("unknown argument: {unknown}"));
            }
        }

        index += 1;
    }

    Ok(cli)
}

fn print_usage() {
    eprintln!(
        "Usage: sentia-health [--daemon|--json] [--from-socket] [--socket-path PATH] \\\n         [--min-interval-ms N] [--interval-ms N] [--max-interval-ms N] [--max-kernel-errors N]"
    );
}

fn write_stdout_line(text: &str) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    stdout.write_all(text.as_bytes())?;
    stdout.write_all(b"\n")?;
    stdout.flush()
}
