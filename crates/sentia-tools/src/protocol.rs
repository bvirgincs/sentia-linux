use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::tools::commands::{
    command_exists, command_help, command_history, command_lookup, CommandExistsInput,
    CommandHelpInput, CommandHistoryInput, CommandLookupInput,
};
use crate::tools::filesystem::{
    directory_contents, disk_usage, file_exists, file_permissions, file_type, DirectoryContentsInput,
    DiskUsageInput, FilePathInput,
};
use crate::tools::logs::{
    journal_errors, journal_recent, kernel_logs, JournalErrorsInput, JournalRecentInput,
    KernelLogsInput,
};
use crate::tools::network::{
    listening_ports, network_interfaces, network_status, ListeningPortsInput,
    NetworkInterfacesInput, NetworkStatusInput,
};
use crate::tools::packages::{package_owns_file, PackageOwnsFileInput};
use crate::tools::processes::{
    process_details, process_list, ProcessDetailsInput, ProcessListInput,
};
use crate::tools::services::{
    service_list, service_status, ServiceListInput, ServiceStatusInput,
};
use crate::tools::system::{
    hardware_information, os_information, HardwareInformationInput, OsInformationInput,
};
use crate::{PrivacyClass, PrivilegeClass, ToolError, ToolResult};

pub const READ_ONLY_PROTOCOL_VERSION: &str = "sentia.tools.readonly.v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolDescriptor {
    pub name: String,
    pub read_only: bool,
    pub privilege_class: PrivilegeClass,
    pub privacy_class: PrivacyClass,
    pub default_timeout_ms: u64,
    pub supports_cancellation: bool,
    pub input_schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolErrorCategory {
    InvalidInput,
    NotFound,
    PermissionDenied,
    Unavailable,
    Timeout,
    OutputLimitExceeded,
    Io,
    Parse,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolErrorEnvelope {
    pub category: ToolErrorCategory,
    pub retryable: bool,
    pub error: ToolError,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ToolRequest {
    List {
        protocol_version: String,
        request_id: Option<String>,
    },
    Invoke {
        protocol_version: String,
        request_id: Option<String>,
        call_id: String,
        tool_name: String,
        input: Value,
    },
    Cancel {
        protocol_version: String,
        request_id: Option<String>,
        call_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ToolResponse {
    List {
        protocol_version: String,
        request_id: Option<String>,
        tools: Vec<ToolDescriptor>,
    },
    Result {
        protocol_version: String,
        request_id: Option<String>,
        call_id: String,
        result: ToolResult,
    },
    Cancelled {
        protocol_version: String,
        request_id: Option<String>,
        call_id: String,
        accepted: bool,
        reason: String,
    },
    Error {
        protocol_version: String,
        request_id: Option<String>,
        call_id: Option<String>,
        error: ToolErrorEnvelope,
    },
}

pub fn handle_request(request: ToolRequest) -> ToolResponse {
    match request {
        ToolRequest::List {
            protocol_version,
            request_id,
        } => {
            if let Err(err) = validate_protocol_version(&protocol_version) {
                return error_response(request_id, None, err);
            }

            ToolResponse::List {
                protocol_version: READ_ONLY_PROTOCOL_VERSION.to_string(),
                request_id,
                tools: readonly_tool_descriptors(),
            }
        }
        ToolRequest::Invoke {
            protocol_version,
            request_id,
            call_id,
            tool_name,
            input,
        } => {
            if let Err(err) = validate_protocol_version(&protocol_version) {
                return error_response(request_id, Some(call_id), err);
            }

            match invoke_tool(&tool_name, input) {
                Ok(result) => ToolResponse::Result {
                    protocol_version: READ_ONLY_PROTOCOL_VERSION.to_string(),
                    request_id,
                    call_id,
                    result,
                },
                Err(err) => error_response(request_id, Some(call_id), err),
            }
        }
        ToolRequest::Cancel {
            protocol_version,
            request_id,
            call_id,
        } => {
            if let Err(err) = validate_protocol_version(&protocol_version) {
                return error_response(request_id, Some(call_id), err);
            }

            ToolResponse::Cancelled {
                protocol_version: READ_ONLY_PROTOCOL_VERSION.to_string(),
                request_id,
                call_id,
                accepted: false,
                reason:
                    "in-flight cancellation is not supported by synchronous library calls; cancel before dispatch"
                        .to_string(),
            }
        }
    }
}

pub fn readonly_tool_names() -> Vec<&'static str> {
    vec![
        "command_exists",
        "command_lookup",
        "command_help",
        "command_history",
        "package_owns_file",
        "service_list",
        "service_status",
        "journal_recent",
        "journal_errors",
        "kernel_logs",
        "network_status",
        "network_interfaces",
        "listening_ports",
        "process_list",
        "process_details",
        "file_exists",
        "file_type",
        "file_permissions",
        "directory_contents",
        "disk_usage",
        "os_information",
        "hardware_information",
    ]
}

pub fn readonly_tool_descriptors() -> Vec<ToolDescriptor> {
    readonly_tool_names()
        .into_iter()
        .filter_map(|name| {
            tool_input_schema(name).map(|schema| ToolDescriptor {
                name: name.to_string(),
                read_only: true,
                privilege_class: PrivilegeClass::UserReadOnly,
                privacy_class: tool_privacy_class(name),
                default_timeout_ms: tool_default_timeout_ms(name),
                supports_cancellation: false,
                input_schema: schema,
            })
        })
        .collect()
}

pub fn tool_input_schema(tool_name: &str) -> Option<Value> {
    match tool_name {
        "command_exists" | "command_lookup" => Some(schema_command_token()),
        "command_help" => Some(json!({
            "type": "object",
            "required": ["command"],
            "additionalProperties": false,
            "properties": {
                "command": { "type": "string", "minLength": 1, "maxLength": 80, "pattern": "^[A-Za-z0-9._+\\-]+$" },
                "section": { "type": ["string", "null"], "minLength": 1, "maxLength": 16, "pattern": "^[A-Za-z0-9.\\-]+$" },
                "max_chars": { "type": ["integer", "null"], "minimum": 1, "maximum": 60000 }
            }
        })),
        "command_history" => Some(json!({
            "type": "object",
            "required": ["entries"],
            "additionalProperties": false,
            "properties": {
                "entries": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 2000,
                    "items": {
                        "type": "object",
                        "required": ["command"],
                        "additionalProperties": false,
                        "properties": {
                            "command": { "type": "string", "minLength": 1, "maxLength": 4096 },
                            "cwd": { "type": ["string", "null"], "pattern": "^/" },
                            "exit_code": { "type": ["integer", "null"] },
                            "timestamp": { "type": ["string", "null"] }
                        }
                    }
                },
                "query": { "type": ["string", "null"], "maxLength": 512 },
                "limit": { "type": ["integer", "null"], "minimum": 1, "maximum": 500 },
                "failures_only": { "type": "boolean" }
            }
        })),
        "package_owns_file" => Some(schema_path_with_sensitive_opt_in()),
        "service_list" => Some(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "limit": { "type": ["integer", "null"], "minimum": 1, "maximum": 2000 },
                "include_inactive": { "type": "boolean" }
            }
        })),
        "service_status" => Some(json!({
            "type": "object",
            "required": ["service"],
            "additionalProperties": false,
            "properties": {
                "service": { "type": "string", "minLength": 1, "maxLength": 128, "pattern": "^[A-Za-z0-9._@\\-]+$" }
            }
        })),
        "journal_recent" => Some(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "lines": { "type": ["integer", "null"], "minimum": 1, "maximum": 1000 },
                "unit": { "type": ["string", "null"], "minLength": 1, "maxLength": 128, "pattern": "^[A-Za-z0-9._@\\-]+$" }
            }
        })),
        "journal_errors" | "kernel_logs" => Some(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "lines": { "type": ["integer", "null"], "minimum": 1, "maximum": 1000 }
            }
        })),
        "network_status" | "network_interfaces" | "os_information" | "hardware_information" => {
            Some(schema_empty_input())
        }
        "listening_ports" => Some(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "limit": { "type": ["integer", "null"], "minimum": 1, "maximum": 2000 },
                "include_udp": { "type": "boolean" }
            }
        })),
        "process_list" => Some(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "limit": { "type": ["integer", "null"], "minimum": 1, "maximum": 2000 },
                "include_cmdline": { "type": "boolean" }
            }
        })),
        "process_details" => Some(json!({
            "type": "object",
            "required": ["pid"],
            "additionalProperties": false,
            "properties": {
                "pid": { "type": "integer", "minimum": 1 },
                "include_cmdline": { "type": "boolean" }
            }
        })),
        "file_exists" | "file_type" | "file_permissions" | "disk_usage" => {
            Some(schema_path_with_sensitive_opt_in())
        }
        "directory_contents" => Some(json!({
            "type": "object",
            "required": ["path"],
            "additionalProperties": false,
            "properties": {
                "path": { "type": "string", "pattern": "^/" },
                "limit": { "type": ["integer", "null"], "minimum": 1, "maximum": 2000 },
                "include_hidden": { "type": "boolean" },
                "allow_sensitive": { "type": "boolean" }
            }
        })),
        _ => None,
    }
}

pub fn invoke_tool(tool_name: &str, input: Value) -> Result<ToolResult, ToolError> {
    match tool_name {
        "command_exists" => command_exists(parse_input::<CommandExistsInput>(tool_name, input)?),
        "command_lookup" => command_lookup(parse_input::<CommandLookupInput>(tool_name, input)?),
        "command_help" => command_help(parse_input::<CommandHelpInput>(tool_name, input)?),
        "command_history" => command_history(parse_input::<CommandHistoryInput>(tool_name, input)?),
        "package_owns_file" => {
            package_owns_file(parse_input::<PackageOwnsFileInput>(tool_name, input)?)
        }
        "service_list" => service_list(parse_input::<ServiceListInput>(tool_name, input)?),
        "service_status" => service_status(parse_input::<ServiceStatusInput>(tool_name, input)?),
        "journal_recent" => journal_recent(parse_input::<JournalRecentInput>(tool_name, input)?),
        "journal_errors" => journal_errors(parse_input::<JournalErrorsInput>(tool_name, input)?),
        "kernel_logs" => kernel_logs(parse_input::<KernelLogsInput>(tool_name, input)?),
        "network_status" => {
            validate_empty_input(tool_name, &input)?;
            network_status(NetworkStatusInput)
        }
        "network_interfaces" => {
            validate_empty_input(tool_name, &input)?;
            network_interfaces(NetworkInterfacesInput)
        }
        "listening_ports" => listening_ports(parse_input::<ListeningPortsInput>(tool_name, input)?),
        "process_list" => process_list(parse_input::<ProcessListInput>(tool_name, input)?),
        "process_details" => process_details(parse_input::<ProcessDetailsInput>(tool_name, input)?),
        "file_exists" => file_exists(parse_input::<FilePathInput>(tool_name, input)?),
        "file_type" => file_type(parse_input::<FilePathInput>(tool_name, input)?),
        "file_permissions" => file_permissions(parse_input::<FilePathInput>(tool_name, input)?),
        "directory_contents" => {
            directory_contents(parse_input::<DirectoryContentsInput>(tool_name, input)?)
        }
        "disk_usage" => disk_usage(parse_input::<DiskUsageInput>(tool_name, input)?),
        "os_information" => {
            validate_empty_input(tool_name, &input)?;
            os_information(OsInformationInput)
        }
        "hardware_information" => {
            validate_empty_input(tool_name, &input)?;
            hardware_information(HardwareInformationInput)
        }
        _ => Err(ToolError::not_found(format!(
            "unknown read-only tool: {tool_name}"
        ))),
    }
}

fn validate_protocol_version(version: &str) -> Result<(), ToolError> {
    if version == READ_ONLY_PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(ToolError::invalid_input(format!(
            "unsupported protocol_version '{}', expected '{}'",
            version, READ_ONLY_PROTOCOL_VERSION
        )))
    }
}

fn validate_empty_input(tool_name: &str, input: &Value) -> Result<(), ToolError> {
    match input {
        Value::Null => Ok(()),
        Value::Object(map) if map.is_empty() => Ok(()),
        _ => Err(ToolError::invalid_input(format!(
            "{tool_name} expects null or empty object input"
        ))),
    }
}

fn parse_input<T: DeserializeOwned>(tool_name: &str, input: Value) -> Result<T, ToolError> {
    serde_json::from_value(input)
        .map_err(|err| ToolError::invalid_input(format!("invalid input for {tool_name}: {err}")))
}

fn tool_privacy_class(tool_name: &str) -> PrivacyClass {
    match tool_name {
        "command_exists" | "command_lookup" => PrivacyClass::PublicMetadata,
        "command_history"
        | "journal_recent"
        | "journal_errors"
        | "kernel_logs"
        | "process_list"
        | "process_details"
        | "file_exists"
        | "file_type"
        | "file_permissions"
        | "directory_contents" => PrivacyClass::PotentiallySensitive,
        _ => PrivacyClass::SystemMetadata,
    }
}

fn tool_default_timeout_ms(tool_name: &str) -> u64 {
    match tool_name {
        "command_exists"
        | "command_lookup"
        | "command_history"
        | "file_exists"
        | "file_type"
        | "file_permissions"
        | "os_information"
        | "hardware_information" => 1_000,
        "network_status"
        | "network_interfaces"
        | "listening_ports"
        | "process_list"
        | "process_details"
        | "directory_contents" => 2_000,
        _ => 4_000,
    }
}

fn schema_empty_input() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {}
    })
}

fn schema_command_token() -> Value {
    json!({
        "type": "object",
        "required": ["command"],
        "additionalProperties": false,
        "properties": {
            "command": { "type": "string", "minLength": 1, "maxLength": 80, "pattern": "^[A-Za-z0-9._+\\-]+$" }
        }
    })
}

fn schema_path_with_sensitive_opt_in() -> Value {
    json!({
        "type": "object",
        "required": ["path"],
        "additionalProperties": false,
        "properties": {
            "path": { "type": "string", "pattern": "^/" },
            "allow_sensitive": { "type": "boolean" }
        }
    })
}

fn error_response(
    request_id: Option<String>,
    call_id: Option<String>,
    error: ToolError,
) -> ToolResponse {
    ToolResponse::Error {
        protocol_version: READ_ONLY_PROTOCOL_VERSION.to_string(),
        request_id,
        call_id,
        error: ToolErrorEnvelope::from_error(error),
    }
}

impl ToolErrorEnvelope {
    pub fn from_error(error: ToolError) -> Self {
        let category = match &error {
            ToolError::InvalidInput { .. } => ToolErrorCategory::InvalidInput,
            ToolError::NotFound { .. } => ToolErrorCategory::NotFound,
            ToolError::PermissionDenied { .. } => ToolErrorCategory::PermissionDenied,
            ToolError::Unavailable { .. } => ToolErrorCategory::Unavailable,
            ToolError::Timeout { .. } => ToolErrorCategory::Timeout,
            ToolError::OutputLimitExceeded { .. } => ToolErrorCategory::OutputLimitExceeded,
            ToolError::Io { .. } => ToolErrorCategory::Io,
            ToolError::Parse { .. } => ToolErrorCategory::Parse,
        };

        let retryable = matches!(
            error,
            ToolError::Unavailable { .. }
                | ToolError::Timeout { .. }
                | ToolError::Io { .. }
                | ToolError::OutputLimitExceeded { .. }
        );

        Self {
            category,
            retryable,
            error,
        }
    }
}
