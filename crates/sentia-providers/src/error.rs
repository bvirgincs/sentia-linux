// SPDX-License-Identifier: Apache-2.0

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderErrorKind {
    Network,
    Dns,
    Authentication,
    AuthenticationExpired,
    Quota,
    UnsupportedCapability,
    Timeout,
    Cancelled,
    MalformedResponse,
    ProcessFailure,
    InvalidRequest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderError {
    pub kind: ProviderErrorKind,
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl ProviderError {
    pub fn new(
        kind: ProviderErrorKind,
        code: impl Into<String>,
        message: impl Into<String>,
        retryable: bool,
    ) -> Self {
        Self {
            kind,
            code: code.into(),
            message: message.into(),
            retryable,
        }
    }

    pub fn invalid(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(
            ProviderErrorKind::InvalidRequest,
            code,
            message,
            false,
        )
    }

    pub fn unsupported(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(
            ProviderErrorKind::UnsupportedCapability,
            code,
            message,
            false,
        )
    }

    pub fn malformed(code: impl Into<String>) -> Self {
        Self::new(
            ProviderErrorKind::MalformedResponse,
            code,
            "The provider returned a response that did not match the reviewed contract.",
            false,
        )
    }

    pub fn process(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ProviderErrorKind::ProcessFailure, code, message, false)
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ProviderError {}
