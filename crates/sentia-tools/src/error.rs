use std::fmt::{Display, Formatter};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolError {
    InvalidInput {
        message: String,
    },
    NotFound {
        message: String,
    },
    PermissionDenied {
        message: String,
    },
    Unavailable {
        message: String,
    },
    Timeout {
        operation: String,
        timeout_secs: u64,
    },
    OutputLimitExceeded {
        stream: String,
        limit_bytes: usize,
    },
    Io {
        context: String,
        message: String,
    },
    Parse {
        source: String,
        message: String,
    },
}

impl ToolError {
    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::InvalidInput {
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound {
            message: message.into(),
        }
    }

    pub fn permission_denied(message: impl Into<String>) -> Self {
        Self::PermissionDenied {
            message: message.into(),
        }
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::Unavailable {
            message: message.into(),
        }
    }

    pub fn timeout(operation: impl Into<String>, timeout_secs: u64) -> Self {
        Self::Timeout {
            operation: operation.into(),
            timeout_secs,
        }
    }

    pub fn output_limit_exceeded(stream: impl Into<String>, limit_bytes: usize) -> Self {
        Self::OutputLimitExceeded {
            stream: stream.into(),
            limit_bytes,
        }
    }

    pub fn io(context: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Io {
            context: context.into(),
            message: message.into(),
        }
    }

    pub fn parse(source: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Parse {
            source: source.into(),
            message: message.into(),
        }
    }
}

impl Display for ToolError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolError::InvalidInput { message } => write!(f, "invalid input: {message}"),
            ToolError::NotFound { message } => write!(f, "not found: {message}"),
            ToolError::PermissionDenied { message } => write!(f, "permission denied: {message}"),
            ToolError::Unavailable { message } => write!(f, "unavailable: {message}"),
            ToolError::Timeout {
                operation,
                timeout_secs,
            } => write!(f, "timeout after {timeout_secs}s while running {operation}"),
            ToolError::OutputLimitExceeded {
                stream,
                limit_bytes,
            } => write!(f, "{stream} output exceeded {limit_bytes} bytes"),
            ToolError::Io { context, message } => write!(f, "io error ({context}): {message}"),
            ToolError::Parse { source, message } => write!(f, "parse error ({source}): {message}"),
        }
    }
}

impl std::error::Error for ToolError {}
