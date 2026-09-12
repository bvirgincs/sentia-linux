use crate::api::{SettingsPatch, UserSettings};
use sentia_protocol::PROTOCOL_VERSION_V1;
use std::{
    env, fs,
    io::{self, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct SettingsStore {
    path: Option<PathBuf>,
    value: Arc<RwLock<UserSettings>>,
}

impl SettingsStore {
    pub fn load_default() -> io::Result<Self> {
        let path = settings_path()?;
        Self::load(path)
    }

    pub fn load(path: PathBuf) -> io::Result<Self> {
        let value = if path.exists() {
            reject_symlink(&path)?;
            let metadata = fs::metadata(&path)?;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "router settings must not be accessible by group or others",
                ));
            }
            let parsed: UserSettings = serde_json::from_slice(&fs::read(&path)?)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            if parsed.version != PROTOCOL_VERSION_V1 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unsupported settings version",
                ));
            }
            parsed
        } else {
            UserSettings::default()
        };
        Ok(Self {
            path: Some(path),
            value: Arc::new(RwLock::new(value)),
        })
    }

    #[cfg(test)]
    pub fn memory(value: UserSettings) -> Self {
        Self {
            path: None,
            value: Arc::new(RwLock::new(value)),
        }
    }

    pub async fn get(&self) -> UserSettings {
        self.value.read().await.clone()
    }

    pub async fn update(&self, patch: SettingsPatch) -> io::Result<UserSettings> {
        let mut guard = self.value.write().await;
        let mut updated = guard.clone();
        if let Some(policy) = patch.default_policy {
            updated.default_policy = policy;
        }
        if let Some(provider) = patch.preferred_remote_provider {
            updated.preferred_remote_provider = match provider {
                Some(value) => {
                    let value = value.trim();
                    if value.is_empty() || value.len() > 48 {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "provider identifier must contain 1 to 48 bytes",
                        ));
                    }
                    Some(value.to_owned())
                }
                None => None,
            };
        }
        if let Some(categories) = patch.remote_categories {
            updated.remote_categories = categories;
        }
        if let Some(path) = &self.path {
            write_private_json(path, &updated)?;
        }
        *guard = updated.clone();
        Ok(updated)
    }
}

fn settings_path() -> io::Result<PathBuf> {
    if let Some(path) = env::var_os("SENTIA_ROUTER_SETTINGS") {
        return Ok(path.into());
    }
    let base = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
    Ok(base.join("sentia/router.json"))
}

fn reject_symlink(path: &Path) -> io::Result<()> {
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "refusing symlink settings file",
        ));
    }
    Ok(())
}

fn write_private_json(path: &Path, value: &UserSettings) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "settings path has no parent"))?;
    if parent.exists() && fs::symlink_metadata(parent)?.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "refusing symlink settings directory",
        ));
    }
    fs::create_dir_all(parent)?;
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    if path.exists() {
        reject_symlink(path)?;
    }
    let temporary = parent.join(format!(".router.json.{}.new", std::process::id()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temporary)?;
        serde_json::to_writer_pretty(&mut file, value)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
