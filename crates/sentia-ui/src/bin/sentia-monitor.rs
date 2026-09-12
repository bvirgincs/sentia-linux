#[cfg(feature = "native-ui")]
use clap::Parser;
#[cfg(feature = "native-ui")]
use gtk4 as gtk;
#[cfg(feature = "native-ui")]
use gtk4::prelude::*;
#[cfg(feature = "native-ui")]
use sentia_ui::monitor::{CapabilityState, CapabilityValue, HealthSnapshot};
#[cfg(feature = "native-ui")]
use std::path::PathBuf;
#[cfg(feature = "native-ui")]
use std::time::Duration;

#[cfg(feature = "native-ui")]
#[derive(Debug, Clone, Parser)]
#[command(name = "sentia-monitor", about = "Sentia system monitor")]
struct Cli {
    #[arg(long)]
    socket: Option<PathBuf>,

    #[arg(long, default_value_t = 2_000)]
    refresh_ms: u64,

    #[arg(long)]
    quit_after_ms: Option<u64>,
}

#[cfg(feature = "native-ui")]
fn main() {
    let cli = Cli::parse();

    let app = gtk::Application::builder()
        .application_id("io.sentia.Monitor")
        .build();

    app.connect_activate(move |application| build_ui(application, cli.clone()));
    app.run();
}

#[cfg(feature = "native-ui")]
fn build_ui(application: &gtk::Application, cli: Cli) {
    let window = gtk::ApplicationWindow::builder()
        .application(application)
        .title("Sentia Monitor")
        .default_width(600)
        .default_height(460)
        .build();

    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.set_margin_top(10);
    root.set_margin_bottom(10);
    root.set_margin_start(10);
    root.set_margin_end(10);

    let title = gtk::Label::new(Some("Sentia health metrics"));
    title.set_xalign(0.0);
    root.append(&title);

    let metrics_label = gtk::Label::new(None);
    metrics_label.set_xalign(0.0);
    metrics_label.set_yalign(0.0);
    metrics_label.set_selectable(true);
    metrics_label.set_wrap(true);
    metrics_label.set_vexpand(true);
    root.append(&metrics_label);

    let footer = gtk::Label::new(Some("Capabilities marked unsupported/unavailable are expected on some hardware."));
    footer.set_xalign(0.0);
    root.append(&footer);

    window.set_child(Some(&root));
    window.present();

    let socket = cli.socket.clone();
    let label_for_refresh = metrics_label.clone();

    refresh_metrics(&label_for_refresh, socket.as_deref());

    glib::source::timeout_add_local(Duration::from_millis(cli.refresh_ms), move || {
        refresh_metrics(&label_for_refresh, socket.as_deref());
        glib::source::Continue(true)
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
fn refresh_metrics(label: &gtk::Label, socket: Option<&std::path::Path>) {
    let snapshot = HealthSnapshot::collect(socket);
    label.set_label(&format_snapshot(&snapshot));
}

#[cfg(feature = "native-ui")]
fn format_snapshot(snapshot: &HealthSnapshot) -> String {
    let mut out = String::new();
    out.push_str(&format!("source: {}\n", snapshot.source));
    out.push_str(&format!("collected_at: {}\n\n", snapshot.collected_at));

    out.push_str(&format!("CPU load: {}\n", describe(&snapshot.cpu_load, |v| {
        format!("1m={:.2} 5m={:.2} 15m={:.2}", v.one, v.five, v.fifteen)
    })));

    out.push_str(&format!("Memory: {}\n", describe(&snapshot.memory, |v| {
        format!("available={} MiB / total={} MiB", v.available_kib / 1024, v.total_kib / 1024)
    })));

    out.push_str(&format!("Swap: {}\n", describe(&snapshot.swap, |v| {
        format!("free={} MiB / total={} MiB", v.free_kib / 1024, v.total_kib / 1024)
    })));

    out.push_str(&format!("Root capacity: {}\n", describe(&snapshot.root_capacity, |v| {
        format!("used={} / total={} ({:.1}%)", v.used_bytes, v.total_bytes, v.used_percent)
    })));

    out.push_str(&format!("Disk health: {}\n", describe(&snapshot.disk_health, Clone::clone)));

    out.push_str(&format!("Network totals: {}\n", describe(&snapshot.network, |v| {
        format!("rx={} bytes tx={} bytes", v.rx_bytes, v.tx_bytes)
    })));

    out.push_str(&format!("Temperature: {}\n", describe(&snapshot.temperature_celsius, |v| {
        format!("{v:.1} °C")
    })));

    out
}

#[cfg(feature = "native-ui")]
fn describe<T>(value: &CapabilityValue<T>, render: impl Fn(&T) -> String) -> String {
    match value.state {
        CapabilityState::Available => value
            .value
            .as_ref()
            .map(render)
            .unwrap_or_else(|| "available (value missing)".to_string()),
        CapabilityState::Unsupported => format!(
            "unsupported{}",
            value
                .note
                .as_ref()
                .map(|note| format!(": {note}"))
                .unwrap_or_default()
        ),
        CapabilityState::Unavailable => format!(
            "unavailable{}",
            value
                .note
                .as_ref()
                .map(|note| format!(": {note}"))
                .unwrap_or_default()
        ),
    }
}

#[cfg(not(feature = "native-ui"))]
fn main() {
    eprintln!("sentia-monitor requires the native-ui Cargo feature (GTK4)");
    std::process::exit(2);
}
