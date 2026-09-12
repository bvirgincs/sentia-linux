#[cfg(feature = "native-ui")]
use clap::Parser;
#[cfg(feature = "native-ui")]
use gtk4 as gtk;
#[cfg(feature = "native-ui")]
use gtk4::prelude::*;
#[cfg(feature = "native-ui")]
use sentia_ui::firstboot::{default_config_path, FirstBootConfig, PrivacyCategories};
#[cfg(feature = "native-ui")]
use sentia_ui::terminal::AssistanceMode;
#[cfg(feature = "native-ui")]
use sentia_ui::transport::RouterPolicy;
#[cfg(feature = "native-ui")]
use std::path::PathBuf;
#[cfg(feature = "native-ui")]
use std::time::Duration;

#[cfg(feature = "native-ui")]
#[derive(Debug, Clone, Parser)]
#[command(name = "sentia-firstboot", about = "Optional offline Sentia setup wizard")]
struct Cli {
    #[arg(long)]
    config: Option<PathBuf>,

    #[arg(long)]
    quit_after_ms: Option<u64>,
}

#[cfg(feature = "native-ui")]
fn main() {
    let cli = Cli::parse();
    let app = gtk::Application::builder()
        .application_id("io.sentia.FirstBoot")
        .build();

    app.connect_activate(move |application| build_ui(application, cli.clone()));
    app.run();
}

#[cfg(feature = "native-ui")]
fn build_ui(application: &gtk::Application, cli: Cli) {
    let config_path = cli.config.clone().unwrap_or_else(default_config_path);
    let existing = FirstBootConfig::load_or_default(Some(config_path.clone()));

    let window = gtk::ApplicationWindow::builder()
        .application(application)
        .title("Sentia first boot")
        .default_width(680)
        .default_height(560)
        .build();

    let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
    root.set_margin_top(12);
    root.set_margin_bottom(12);
    root.set_margin_start(12);
    root.set_margin_end(12);

    let intro = gtk::Label::new(Some(
        "This optional wizard is fully offline and skippable. Defaults are LOCAL_ONLY with telemetry disabled.",
    ));
    intro.set_wrap(true);
    intro.set_xalign(0.0);
    root.append(&intro);

    let policy_combo = gtk::ComboBoxText::new();
    policy_combo.append(Some("LOCAL_ONLY"), "LOCAL_ONLY");
    policy_combo.append(Some("LOCAL_PREFERRED"), "LOCAL_PREFERRED");
    policy_combo.append(Some("REMOTE_PREFERRED"), "REMOTE_PREFERRED");
    policy_combo.append(Some("ASK_BEFORE_REMOTE"), "ASK_BEFORE_REMOTE");
    policy_combo.set_active_id(Some(existing.policy.as_contract_label()));

    root.append(&labeled("Routing policy", &policy_combo));

    let terminal_combo = gtk::ComboBoxText::new();
    terminal_combo.append(Some("command-not-found"), "command-not-found");
    terminal_combo.append(Some("on-request"), "on-request");
    terminal_combo.append(Some("auto-suggest"), "auto-suggest");
    terminal_combo.append(Some("disabled"), "disabled");
    terminal_combo.append(Some("conventional"), "conventional");
    terminal_combo.set_active_id(Some(existing.terminal_mode.env_value()));

    root.append(&labeled("Terminal assistance", &terminal_combo));

    let telemetry_toggle = gtk::CheckButton::with_label("Enable telemetry (disabled by default)");
    telemetry_toggle.set_active(existing.telemetry_enabled);
    root.append(&telemetry_toggle);

    let privacy_title = gtk::Label::new(Some("Privacy categories for future remote requests"));
    privacy_title.set_xalign(0.0);
    root.append(&privacy_title);

    let files = gtk::CheckButton::with_label("Files");
    files.set_active(existing.privacy.files);
    let command_history = gtk::CheckButton::with_label("Command history");
    command_history.set_active(existing.privacy.command_history);
    let hostnames = gtk::CheckButton::with_label("Hostnames");
    hostnames.set_active(existing.privacy.hostnames);
    let ip_addresses = gtk::CheckButton::with_label("IP addresses");
    ip_addresses.set_active(existing.privacy.ip_addresses);
    let process_names = gtk::CheckButton::with_label("Process names");
    process_names.set_active(existing.privacy.process_names);
    let journals = gtk::CheckButton::with_label("Journals");
    journals.set_active(existing.privacy.journals);

    for toggle in [
        &files,
        &command_history,
        &hostnames,
        &ip_addresses,
        &process_names,
        &journals,
    ] {
        root.append(toggle);
    }

    let providers_title = gtk::Label::new(Some(
        "Eligible providers (optional, no direct provider calls are made by this wizard)",
    ));
    providers_title.set_wrap(true);
    providers_title.set_xalign(0.0);
    root.append(&providers_title);

    let provider_openai = gtk::CheckButton::with_label("OpenAI API");
    provider_openai.set_active(existing.eligible_providers.iter().any(|item| item == "openai"));
    let provider_anthropic = gtk::CheckButton::with_label("Anthropic API");
    provider_anthropic
        .set_active(existing.eligible_providers.iter().any(|item| item == "anthropic"));
    let provider_google = gtk::CheckButton::with_label("Google Gemini API");
    provider_google.set_active(existing.eligible_providers.iter().any(|item| item == "google"));
    let provider_xai = gtk::CheckButton::with_label("xAI API");
    provider_xai.set_active(existing.eligible_providers.iter().any(|item| item == "xai"));
    let provider_deepseek = gtk::CheckButton::with_label("DeepSeek API");
    provider_deepseek
        .set_active(existing.eligible_providers.iter().any(|item| item == "deepseek"));

    for toggle in [
        &provider_openai,
        &provider_anthropic,
        &provider_google,
        &provider_xai,
        &provider_deepseek,
    ] {
        root.append(toggle);
    }

    let privileged_notice = gtk::Label::new(Some(
        "Privileged actions are intentionally absent here; any future root mutation must show a canonical plan and require typed confirmation via the broker before polkit.",
    ));
    privileged_notice.set_wrap(true);
    privileged_notice.set_xalign(0.0);
    root.append(&privileged_notice);

    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    root.append(&status);

    let button_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let skip_button = gtk::Button::with_label("Skip");
    let save_button = gtk::Button::with_label("Save");
    button_row.append(&skip_button);
    button_row.append(&save_button);
    root.append(&button_row);

    window.set_child(Some(&root));
    window.present();

    let status_for_skip = status.clone();
    let path_for_skip = config_path.clone();
    skip_button.connect_clicked(move |_| {
        let mut config = FirstBootConfig::default();
        config.wizard_skipped = true;
        config.completed = true;
        match config.save(Some(path_for_skip.clone())) {
            Ok(path) => status_for_skip.set_label(&format!("Skipped. Saved defaults to {}", path.display())),
            Err(error) => status_for_skip.set_label(&format!("Failed to save defaults: {error}")),
        }
    });

    let status_for_save = status.clone();
    let path_for_save = config_path.clone();
    save_button.connect_clicked(move |_| {
        let policy = match policy_combo.active_id().as_deref() {
            Some("LOCAL_PREFERRED") => RouterPolicy::LocalPreferred,
            Some("REMOTE_PREFERRED") => RouterPolicy::RemotePreferred,
            Some("ASK_BEFORE_REMOTE") => RouterPolicy::AskBeforeRemote,
            _ => RouterPolicy::LocalOnly,
        };

        let terminal_mode = match terminal_combo.active_id().as_deref() {
            Some("on-request") => AssistanceMode::OnRequest,
            Some("auto-suggest") => AssistanceMode::AutoSuggest,
            Some("disabled") => AssistanceMode::Disabled,
            Some("conventional") => AssistanceMode::Conventional,
            _ => AssistanceMode::CommandNotFound,
        };

        let mut providers = Vec::new();
        if provider_openai.is_active() {
            providers.push("openai".to_string());
        }
        if provider_anthropic.is_active() {
            providers.push("anthropic".to_string());
        }
        if provider_google.is_active() {
            providers.push("google".to_string());
        }
        if provider_xai.is_active() {
            providers.push("xai".to_string());
        }
        if provider_deepseek.is_active() {
            providers.push("deepseek".to_string());
        }

        let config = FirstBootConfig {
            wizard_skipped: false,
            completed: true,
            policy,
            telemetry_enabled: telemetry_toggle.is_active(),
            terminal_mode,
            privacy: PrivacyCategories {
                files: files.is_active(),
                command_history: command_history.is_active(),
                hostnames: hostnames.is_active(),
                ip_addresses: ip_addresses.is_active(),
                process_names: process_names.is_active(),
                journals: journals.is_active(),
            },
            eligible_providers: providers,
            updated_at: format!(
                "unix:{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|duration| duration.as_secs())
                    .unwrap_or_default()
            ),
        };

        match config.save(Some(path_for_save.clone())) {
            Ok(path) => status_for_save.set_label(&format!("Saved {}", path.display())),
            Err(error) => status_for_save.set_label(&format!("Save failed: {error}")),
        }
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
fn labeled(label: &str, widget: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let heading = gtk::Label::new(Some(label));
    heading.set_xalign(0.0);
    row.append(&heading);
    row.append(widget);
    row
}

#[cfg(not(feature = "native-ui"))]
fn main() {
    eprintln!("sentia-firstboot requires the native-ui Cargo feature (GTK4)");
    std::process::exit(2);
}
