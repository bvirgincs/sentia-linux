use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

#[derive(Clone, Copy)]
enum StopClass {
    Question,
    Polite,
    Task,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum AssistanceMode {
    CommandNotFound,
    OnRequest,
    AutoSuggest,
    Disabled,
    Conventional,
}

impl AssistanceMode {
    pub fn env_value(self) -> &'static str {
        match self {
            AssistanceMode::CommandNotFound => "command-not-found",
            AssistanceMode::OnRequest => "on-request",
            AssistanceMode::AutoSuggest => "auto-suggest",
            AssistanceMode::Disabled => "disabled",
            AssistanceMode::Conventional => "conventional",
        }
    }
}

impl Default for AssistanceMode {
    fn default() -> Self {
        AssistanceMode::CommandNotFound
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CommandNotFoundClass {
    LikelyTypo,
    MissingPackage,
    NaturalLanguage,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandNotFoundResult {
    pub class: CommandNotFoundClass,
    pub command: String,
    pub suggestion: Option<String>,
    pub packages: Vec<String>,
    pub ai_hint: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PackageIndex {
    map: BTreeMap<String, BTreeSet<String>>,
}

impl PackageIndex {
    pub fn from_tsv(path: &Path) -> std::io::Result<Self> {
        let raw = fs::read_to_string(path)?;
        let mut map = BTreeMap::<String, BTreeSet<String>>::new();

        for line in raw.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            let mut parts = trimmed.splitn(2, '\t');
            let command = parts.next().unwrap_or_default().trim();
            let package = parts.next().unwrap_or_default().trim();
            if command.is_empty() || package.is_empty() {
                continue;
            }

            map.entry(command.to_string())
                .or_default()
                .insert(package.to_string());
        }

        Ok(Self { map })
    }

    pub fn from_pairs<I, A, B>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (A, B)>,
        A: Into<String>,
        B: Into<String>,
    {
        let mut map = BTreeMap::<String, BTreeSet<String>>::new();
        for (command, package) in pairs {
            map.entry(command.into()).or_default().insert(package.into());
        }
        Self { map }
    }

    pub fn lookup(&self, command: &str) -> Vec<String> {
        self.map
            .get(command)
            .map(|packages| packages.iter().cloned().collect())
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone)]
pub struct ClassifierConfig {
    pub max_typo_distance: usize,
    pub executable_catalog: BTreeSet<String>,
    pub package_index: Option<PackageIndex>,
}

impl Default for ClassifierConfig {
    fn default() -> Self {
        let package_index = std::env::var_os("SENTIA_COMMAND_INDEX")
            .map(|path| PackageIndex::from_tsv(Path::new(&path)).ok())
            .flatten();

        Self {
            max_typo_distance: 2,
            executable_catalog: collect_executables(),
            package_index,
        }
    }
}

pub fn classify_command_line(raw_command_line: &str, config: &ClassifierConfig) -> CommandNotFoundResult {
    let trimmed = raw_command_line.trim();
    let command = trimmed
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string();

    if command.is_empty() {
        return CommandNotFoundResult {
            class: CommandNotFoundClass::Unknown,
            command,
            suggestion: None,
            packages: Vec::new(),
            ai_hint: None,
        };
    }

    if let Some((suggestion, _distance)) = best_typo_match(
        &command,
        &config.executable_catalog,
        config.max_typo_distance,
    ) {
        if suggestion != command {
            return CommandNotFoundResult {
                class: CommandNotFoundClass::LikelyTypo,
                command,
                suggestion: Some(suggestion),
                packages: Vec::new(),
                ai_hint: None,
            };
        }
    }

    if let Some(index) = &config.package_index {
        let packages = index.lookup(&command);
        if !packages.is_empty() {
            return CommandNotFoundResult {
                class: CommandNotFoundClass::MissingPackage,
                command,
                suggestion: None,
                packages,
                ai_hint: None,
            };
        }
    }

    if looks_like_natural_language(trimmed) {
        return CommandNotFoundResult {
            class: CommandNotFoundClass::NaturalLanguage,
            command,
            suggestion: None,
            packages: Vec::new(),
            ai_hint: Some(format!("ai -- {}", shell_quote(trimmed))),
        };
    }

    CommandNotFoundResult {
        class: CommandNotFoundClass::Unknown,
        command,
        suggestion: None,
        packages: Vec::new(),
        ai_hint: None,
    }
}

pub fn collect_executables() -> BTreeSet<String> {
    let search_path = env::var_os("SENTIA_EXECUTABLE_SEARCH_PATH")
        .or_else(|| env::var_os("PATH"))
        .unwrap_or_else(|| "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".into());

    let mut out = BTreeSet::new();

    for path in env::split_paths(&search_path) {
        if !path.is_dir() {
            continue;
        }

        let Ok(entries) = fs::read_dir(path) else {
            continue;
        };

        for entry in entries.flatten() {
            let Ok(metadata) = entry.metadata() else {
                continue;
            };

            if !metadata.is_file() {
                continue;
            }

            if metadata.permissions().mode() & 0o111 == 0 {
                continue;
            }

            if let Some(name) = entry.file_name().to_str() {
                out.insert(name.to_string());
            }
        }
    }

    out
}

fn looks_like_natural_language(raw: &str) -> bool {
    if raw.split_whitespace().count() < 3 {
        return false;
    }

    let lower = raw.to_ascii_lowercase();
    if lower.contains('?') {
        return true;
    }

    let stop_class = [
        ("how", StopClass::Question),
        ("what", StopClass::Question),
        ("why", StopClass::Question),
        ("please", StopClass::Polite),
        ("could", StopClass::Polite),
        ("should", StopClass::Polite),
        ("show", StopClass::Task),
        ("list", StopClass::Task),
        ("find", StopClass::Task),
        ("explain", StopClass::Task),
        ("install", StopClass::Task),
    ];

    let mut seen_question = false;
    let mut seen_other = false;

    for token in lower.split_whitespace() {
        for (stop, class) in stop_class {
            if token == stop {
                match class {
                    StopClass::Question => seen_question = true,
                    StopClass::Polite | StopClass::Task => seen_other = true,
                }
            }
        }
    }

    seen_question || seen_other
}

fn best_typo_match(
    command: &str,
    executable_catalog: &BTreeSet<String>,
    max_distance: usize,
) -> Option<(String, usize)> {
    executable_catalog
        .iter()
        .filter_map(|candidate| {
            let distance = levenshtein(command, candidate);
            (distance <= max_distance).then(|| (candidate.clone(), distance))
        })
        .min_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)))
}

fn levenshtein(a: &str, b: &str) -> usize {
    let b_len = b.chars().count();
    let mut prev: Vec<usize> = (0..=b_len).collect();
    let mut curr = vec![0usize; b_len + 1];

    for (i, ca) in a.chars().enumerate() {
        curr[0] = i + 1;
        for (j, cb) in b.chars().enumerate() {
            let cost = usize::from(ca != cb);
            curr[j + 1] = (curr[j] + 1)
                .min(prev[j + 1] + 1)
                .min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[b_len]
}

fn shell_quote(value: &str) -> String {
    let escaped = value.replace('\'', "'\\''");
    format!("'{}'", escaped)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapturedCommand {
    pub timestamp: String,
    pub cwd: String,
    pub command: String,
    pub exit_status: i32,
    pub combined_output: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CaptureBuffer {
    max_events: usize,
    max_total_output_bytes: usize,
    output_bytes: usize,
    events: VecDeque<CapturedCommand>,
}

impl CaptureBuffer {
    pub fn new(max_events: usize, max_total_output_bytes: usize) -> Self {
        Self {
            max_events,
            max_total_output_bytes,
            output_bytes: 0,
            events: VecDeque::new(),
        }
    }

    pub fn push(&mut self, mut event: CapturedCommand) {
        if let Some(output) = &event.combined_output {
            self.output_bytes += output.len();
        }

        while self.events.len() >= self.max_events {
            if let Some(old) = self.events.pop_front() {
                if let Some(output) = old.combined_output {
                    self.output_bytes = self.output_bytes.saturating_sub(output.len());
                }
            }
        }

        while self.output_bytes > self.max_total_output_bytes {
            let Some(old) = self.events.pop_front() else {
                break;
            };

            if let Some(output) = old.combined_output {
                self.output_bytes = self.output_bytes.saturating_sub(output.len());
            }
        }

        if self.output_bytes > self.max_total_output_bytes {
            event.combined_output = None;
        }

        self.events.push_back(event);
    }

    pub fn events(&self) -> &VecDeque<CapturedCommand> {
        &self.events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typo_is_preferred_over_package_match() {
        let mut executable_catalog = BTreeSet::new();
        executable_catalog.insert("ls".to_string());

        let package_index = PackageIndex::from_pairs([(String::from("sl"), String::from("steam-locomotive"))]);
        let config = ClassifierConfig {
            max_typo_distance: 2,
            executable_catalog,
            package_index: Some(package_index),
        };

        let result = classify_command_line("sl", &config);
        assert_eq!(result.class, CommandNotFoundClass::LikelyTypo);
        assert_eq!(result.suggestion.as_deref(), Some("ls"));
    }

    #[test]
    fn package_lookup_works() {
        let package_index = PackageIndex::from_pairs([(String::from("htop"), String::from("htop"))]);

        let config = ClassifierConfig {
            max_typo_distance: 1,
            executable_catalog: BTreeSet::new(),
            package_index: Some(package_index),
        };

        let result = classify_command_line("htop", &config);
        assert_eq!(result.class, CommandNotFoundClass::MissingPackage);
        assert_eq!(result.packages, vec![String::from("htop")]);
    }

    #[test]
    fn natural_language_routes_to_ai() {
        let config = ClassifierConfig {
            max_typo_distance: 1,
            executable_catalog: BTreeSet::new(),
            package_index: None,
        };

        let result = classify_command_line("how can I list files", &config);
        assert_eq!(result.class, CommandNotFoundClass::NaturalLanguage);
        assert!(result.ai_hint.unwrap().starts_with("ai -- 'how"));
    }

    #[test]
    fn capture_buffer_bounds_events() {
        let mut buffer = CaptureBuffer::new(2, 20);
        for index in 0..3 {
            buffer.push(CapturedCommand {
                timestamp: index.to_string(),
                cwd: "/home/ubuntu".to_string(),
                command: format!("echo {index}"),
                exit_status: 0,
                combined_output: Some("ok".to_string()),
            });
        }

        assert_eq!(buffer.events().len(), 2);
    }
}
