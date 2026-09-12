use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Result, RetrievalError};

#[derive(Debug, Clone)]
pub struct Allowlist {
    roots: Vec<PathBuf>,
}

impl Allowlist {
    pub fn for_root(root: &Path) -> Result<Self> {
        let canonical = fs::canonicalize(root).map_err(|source| RetrievalError::Canonicalize {
            path: root.to_path_buf(),
            source,
        })?;
        Ok(Self {
            roots: vec![canonical],
        })
    }

    pub fn contains_resolved(&self, resolved: &Path) -> bool {
        self.roots.iter().any(|root| resolved.starts_with(root))
    }

    pub fn resolve_if_allowed(&self, candidate: &Path) -> Result<Option<PathBuf>> {
        let resolved = fs::canonicalize(candidate).map_err(|source| RetrievalError::Canonicalize {
            path: candidate.to_path_buf(),
            source,
        })?;

        if self.contains_resolved(&resolved) {
            Ok(Some(resolved))
        } else {
            Ok(None)
        }
    }
}
