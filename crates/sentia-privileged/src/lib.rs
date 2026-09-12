// SPDX-License-Identifier: Apache-2.0
pub mod broker;
pub mod executor;
pub mod service;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

pub const BUS_NAME: &str = "org.sentia.System1";
pub const OBJECT_PATH: &str = "/org/sentia/System1";
pub const PLAN_LIFETIME_SECONDS: u64 = 120;
pub const MAX_REQUEST_BYTES: usize = 16_384;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub &'static str);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ServiceAction {
    Start,
    Stop,
    Restart,
    Enable,
}

impl ServiceAction {
    pub fn argument(&self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
            Self::Enable => "enable",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Service { action: ServiceAction, unit: String },
    ProcessKill { pid: u32 },
    AptInstall { packages: Vec<String> },
    AptRemove { packages: Vec<String> },
    AptUpdate,
    AptUpgrade,
}

impl Operation {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Service { unit, .. } => validate_unit(unit),
            Self::ProcessKill { pid } if *pid <= 1 || *pid > i32::MAX as u32 => {
                Err(Error("invalid_pid"))
            }
            Self::AptInstall { packages } | Self::AptRemove { packages } => {
                if packages.is_empty() || packages.len() > 64 {
                    return Err(Error("invalid_packages"));
                }
                let mut unique = std::collections::BTreeSet::new();
                for package in packages {
                    validate_package(package)?;
                    if !unique.insert(package) {
                        return Err(Error("duplicate_package"));
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    pub fn action_id(&self) -> &'static str {
        match self {
            Self::Service { action, .. } => match action {
                ServiceAction::Start => "org.sentia.system.service-start",
                ServiceAction::Stop => "org.sentia.system.service-stop",
                ServiceAction::Restart => "org.sentia.system.service-restart",
                ServiceAction::Enable => "org.sentia.system.service-enable",
            },
            Self::ProcessKill { .. } => "org.sentia.system.process-terminate",
            Self::AptInstall { .. } => "org.sentia.system.package-install",
            Self::AptRemove { .. } => "org.sentia.system.package-remove",
            Self::AptUpdate => "org.sentia.system.package-update",
            Self::AptUpgrade => "org.sentia.system.package-upgrade",
        }
    }
}

pub fn validate_unit(unit: &str) -> Result<()> {
    // Deliberately narrower than systemd's complete grammar: no templates,
    // escapes, paths, globbing, options, or non-service unit types.
    let stem = unit.strip_suffix(".service").ok_or(Error("invalid_unit"))?;
    if unit.len() > 255
        || stem.is_empty()
        || !stem.as_bytes()[0].is_ascii_alphanumeric()
        || !stem
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
        || unit.contains("..")
    {
        return Err(Error("invalid_unit"));
    }
    Ok(())
}

pub fn validate_package(package: &str) -> Result<()> {
    if package.len() < 2
        || package.len() > 128
        || !package.as_bytes()[0].is_ascii_lowercase()
            && !package.as_bytes()[0].is_ascii_digit()
        || !package
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"+.-".contains(&c))
    {
        return Err(Error("invalid_package"));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareRequest {
    pub version: u32,
    pub operation: Operation,
}

pub fn parse_request(json: &str) -> Result<Operation> {
    if json.len() > MAX_REQUEST_BYTES {
        return Err(Error("request_too_large"));
    }
    let request: PrepareRequest =
        serde_json::from_str(json).map_err(|_| Error("invalid_request"))?;
    if request.version != 1 {
        return Err(Error("unsupported_version"));
    }
    request.operation.validate()?;
    Ok(request.operation)
}

pub fn digest<T: Serialize>(value: &T) -> Result<String> {
    let bytes = serde_json::to_vec(value).map_err(|_| Error("serialization_failed"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
