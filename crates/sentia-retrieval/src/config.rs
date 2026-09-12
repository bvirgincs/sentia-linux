use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{Result, RetrievalError};
use crate::model::SourceKind;

fn default_schema_version() -> u32 {
    1
}

fn default_max_file_count() -> usize {
    10_000
}

fn default_max_file_bytes() -> usize {
    262_144
}

fn default_max_total_bytes() -> usize {
    64 * 1024 * 1024
}

fn default_max_query_results() -> usize {
    8
}

fn default_max_depth() -> usize {
    6
}

#[derive(Debug, Clone, Deserialize)]
pub struct RetrievalConfig {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default = "default_max_file_count")]
    pub max_file_count: usize,
    #[serde(default = "default_max_file_bytes")]
    pub max_file_bytes: usize,
    #[serde(default = "default_max_total_bytes")]
    pub max_total_bytes: usize,
    #[serde(default = "default_max_query_results")]
    pub max_query_results: usize,
    #[serde(default)]
    pub source: Vec<SourceConfig>,
}

impl RetrievalConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path).map_err(|source| RetrievalError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let config: Self = toml::from_str(&text)
            .map_err(|e| RetrievalError::InvalidConfig(format!("{}: {}", path.display(), e)))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            return Err(RetrievalError::InvalidConfig(format!(
                "unsupported schema_version {}; expected 1",
                self.schema_version
            )));
        }

        if self.max_file_count == 0 || self.max_file_bytes == 0 || self.max_total_bytes == 0 {
            return Err(RetrievalError::InvalidConfig(
                "max_file_count, max_file_bytes, and max_total_bytes must be non-zero".into(),
            ));
        }

        let mut names = HashSet::new();
        if self.source.is_empty() {
            return Err(RetrievalError::InvalidConfig(
                "at least one [[source]] entry is required".into(),
            ));
        }

        for source in &self.source {
            if source.name.trim().is_empty() {
                return Err(RetrievalError::InvalidConfig(
                    "source.name cannot be empty".into(),
                ));
            }
            if !names.insert(source.name.clone()) {
                return Err(RetrievalError::InvalidConfig(format!(
                    "duplicate source name {}",
                    source.name
                )));
            }

            match source.kind {
                SourceKindConfig::Filesystem | SourceKindConfig::ManDirectory => {
                    if source.roots.is_empty() {
                        return Err(RetrievalError::InvalidConfig(format!(
                            "source {} requires non-empty roots",
                            source.name
                        )));
                    }
                }
                SourceKindConfig::DpkgStatus => {
                    if source.path.is_none() {
                        return Err(RetrievalError::InvalidConfig(format!(
                            "source {} requires path for dpkg_status",
                            source.name
                        )));
                    }
                }
            }

            if !source.allow_private_roots {
                for candidate in source.all_paths() {
                    if path_is_disallowed(candidate) {
                        return Err(RetrievalError::InvalidConfig(format!(
                            "source {} path {} is not allowed; configure explicit system documentation paths only",
                            source.name,
                            candidate.display()
                        )));
                    }
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKindConfig {
    Filesystem,
    DpkgStatus,
    ManDirectory,
}

impl SourceKindConfig {
    pub fn as_model(self) -> SourceKind {
        match self {
            SourceKindConfig::Filesystem => SourceKind::Filesystem,
            SourceKindConfig::DpkgStatus => SourceKind::DpkgStatus,
            SourceKindConfig::ManDirectory => SourceKind::ManDirectory,
        }
    }

    pub fn default_trust_label(self) -> &'static str {
        match self {
            SourceKindConfig::Filesystem => "PACKAGED_DOCUMENTATION",
            SourceKindConfig::DpkgStatus => "LOCAL_SYSTEM_METADATA",
            SourceKindConfig::ManDirectory => "DEBIAN_MANPAGE",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SourceConfig {
    pub name: String,
    pub kind: SourceKindConfig,
    pub trust_label: Option<String>,
    #[serde(default)]
    pub roots: Vec<PathBuf>,
    pub path: Option<PathBuf>,
    #[serde(default)]
    pub allow_pages: Vec<String>,
    #[serde(default)]
    pub include_extensions: Vec<String>,
    #[serde(default = "default_max_depth")]
    pub max_depth: usize,
    #[serde(default)]
    pub allow_private_roots: bool,
    pub package_name: Option<String>,
    pub package_version: Option<String>,
}

impl SourceConfig {
    pub fn all_paths(&self) -> Vec<&PathBuf> {
        let mut paths = self.roots.iter().collect::<Vec<_>>();
        if let Some(path) = &self.path {
            paths.push(path);
        }
        paths
    }

    pub fn trust_label(&self) -> String {
        self.trust_label
            .clone()
            .unwrap_or_else(|| self.kind.default_trust_label().to_string())
    }

    pub fn configured_roots(&self) -> Vec<String> {
        let mut roots = self
            .roots
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>();
        if let Some(path) = &self.path {
            roots.push(path.display().to_string());
        }
        roots
    }
}

fn path_is_disallowed(path: &Path) -> bool {
    if !path.is_absolute() {
        return false;
    }

    let disallowed_prefixes = [
        Path::new("/home"),
        Path::new("/root"),
        Path::new("/proc"),
        Path::new("/sys/kernel/security"),
        Path::new("/run/user"),
    ];

    if disallowed_prefixes.iter().any(|prefix| path.starts_with(prefix)) {
        return true;
    }

    path.components().any(|component| {
        let text = component.as_os_str().to_string_lossy();
        text == ".ssh"
            || text == ".gnupg"
            || text == ".aws"
            || text == ".env"
            || text.eq_ignore_ascii_case("credentials")
    })
}
