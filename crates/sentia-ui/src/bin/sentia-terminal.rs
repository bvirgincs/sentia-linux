#[cfg(feature = "native-ui")]
use clap::Parser;
#[cfg(feature = "native-ui")]
use gtk4 as gtk;
#[cfg(feature = "native-ui")]
use gtk4::prelude::*;
#[cfg(feature = "native-ui")]
use sentia_ui::terminal::AssistanceMode;
#[cfg(feature = "native-ui")]
use sentia_ui::transport::{health_socket_path, router_socket_path};
#[cfg(feature = "native-ui")]
use std::path::PathBuf;
#[cfg(feature = "native-ui")]
use std::time::Duration;
#[cfg(feature = "native-ui")]
use vte4::prelude::*;

#[cfg(feature = "native-ui")]
#[derive(Debug, Clone, Parser)]
#[command(name = "sentia-terminal", about = "Native Sentia terminal (GTK4 + VTE)")]
struct Cli {
    #[arg(long, default_value_t = AssistanceMode::CommandNotFound, value_enum)]
    assist_mode: AssistanceMode,

    #[arg(long, default_value_t = false)]
    conventional: bool,

    #[arg(long, default_value_t = false)]
    capture: bool,

    #[arg(long, default_value_t = 128)]
    capture_max_entries: usize,

    #[arg(long)]
    cwd: Option<PathBuf>,

    #[arg(long, default_value = "/bin/bash")]
    shell: PathBuf,

    #[arg(long)]
    command: Option<String>,

    #[arg(long)]
    quit_after_ms: Option<u64>,
}

#[cfg(feature = "native-ui")]
fn main() {
    let cli = Cli::parse();

    let app = gtk::Application::builder()
        .application_id("io.sentia.Terminal")
        .build();

    app.connect_activate(move |application| build_ui(application, cli.clone()));

    app.run();
}

#[cfg(feature = "native-ui")]
fn build_ui(application: &gtk::Application, cli: Cli) {
    let window = gtk::ApplicationWindow::builder()
        .application(application)
        .title("Sentia Terminal")
        .default_width(980)
        .default_height(680)
        .build();

    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.set_margin_top(8);
    root.set_margin_bottom(8);
    root.set_margin_start(8);
    root.set_margin_end(8);

    let banner = gtk::Label::new(Some(
        "PTY stream is combined stdout/stderr; terminal output is untrusted data.",
    ));
    banner.set_xalign(0.0);
    root.append(&banner);

    let terminal = vte4::Terminal::new();
    terminal.set_vexpand(true);
    terminal.set_hexpand(true);
    terminal.set_scrollback_lines(20_000);
    terminal.set_allow_hyperlink(false);
    root.append(&terminal);

    let status = gtk::Label::new(Some("Launching /bin/bash..."));
    status.set_xalign(0.0);
    root.append(&status);

    window.set_child(Some(&root));
    window.present();

    if let Err(error) = spawn_shell(&terminal, &status, &cli) {
        status.set_label(&format!("Failed to spawn shell: {error}"));
    }

    let status_clone = status.clone();
    terminal.connect_child_exited(move |_term, status_code| {
        status_clone.set_label(&format!("Shell exited with status {status_code}"));
    });

    if let Some(quit_after_ms) = cli.quit_after_ms {
        let app_clone = application.clone();
        glib::source::timeout_add_local(Duration::from_millis(quit_after_ms), move || {
            app_clone.quit();
            glib::source::Continue(false)
        });
    }
}

#[cfg(feature = "native-ui")]
fn spawn_shell(terminal: &vte4::Terminal, status: &gtk::Label, cli: &Cli) -> anyhow::Result<()> {
    let effective_mode = if cli.conventional {
        AssistanceMode::Conventional
    } else {
        cli.assist_mode
    };

    let mut envv = std::env::vars().collect::<Vec<(String, String)>>();
    envv.push((
        "SENTIA_ROUTER_SOCKET".to_string(),
        router_socket_path().display().to_string(),
    ));
    envv.push((
        "SENTIA_HEALTH_SOCKET".to_string(),
        health_socket_path().display().to_string(),
    ));

    envv.push((
        "SENTIA_TERMINAL_ASSIST_MODE".to_string(),
        effective_mode.env_value().to_string(),
    ));

    if !cli.conventional {
        envv.push(("_SENTIA_TERMINAL".to_string(), "1".to_string()));
    }

    if cli.capture && !cli.conventional {
        envv.push(("SENTIA_TERMINAL_CAPTURE".to_string(), "1".to_string()));
    }

    envv.push((
        "SENTIA_CAPTURE_MAX".to_string(),
        cli.capture_max_entries.to_string(),
    ));

    envv.push((
        "SENTIA_TERMINAL_HOOK".to_string(),
        "/usr/share/sentia/sentia-terminal-hook.bash".to_string(),
    ));

    let envv_joined: Vec<String> = envv
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect();
    let envv_refs: Vec<&str> = envv_joined.iter().map(String::as_str).collect();

    let mut argv = vec![
        cli.shell.display().to_string(),
        "--login".to_string(),
    ];

    if let Some(command) = &cli.command {
        argv.push("-lc".to_string());
        argv.push(command.clone());
    }

    let argv_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    let cwd = cli.cwd.as_ref().map(|path| path.display().to_string());

    let status_label = status.clone();
    terminal.spawn_async(
        vte4::PtyFlags::DEFAULT,
        cwd.as_deref(),
        &argv_refs,
        &envv_refs,
        glib::SpawnFlags::SEARCH_PATH,
        || {},
        -1,
        None::<&gtk::gio::Cancellable>,
        move |spawn_result| {
            if let Err(error) = spawn_result {
                status_label.set_label(&format!("Shell launch error: {error}"));
            } else {
                status_label.set_label("Conventional Bash ready");
            }
        },
    );

    Ok(())
}

#[cfg(not(feature = "native-ui"))]
fn main() {
    eprintln!("sentia-terminal requires the native-ui Cargo feature (GTK4 + VTE)");
    std::process::exit(2);
}
