use async_trait::async_trait;
use sentia_local_broker::protocol::{BrokerTool, BrokerToolCall};
use sentia_protocol::{
    BoundedString, JsonlFrame, PrivilegeClass, ToolName, ToolProvenance, ToolProvenanceSource,
    ToolRequest, ModelToolResultStatus, MAX_TOOL_INPUT_BYTES_V1, PROTOCOL_VERSION_V1,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::{HashMap, HashSet},
    fs, io,
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
    time::{timeout, Duration},
};
use tokio_util::sync::CancellationToken;

const MAX_TOOL_ARGUMENT_BYTES: usize = 64 * 1024;
const MAX_TOOL_RESULT_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolClass {
    ReadOnly,
    ActionProposal,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ModelToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub class: ToolClass,
    pub provenance: String,
}

impl ModelToolDefinition {
    pub fn model_schema(&self) -> BrokerTool {
        BrokerTool {
            name: self.name.clone(),
            description: self.description.clone(),
            parameters: self.input_schema.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ModelToolResult {
    pub content: String,
    pub provenance: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("unknown tool")]
    Unknown,
    #[error("tool arguments are invalid: {0}")]
    InvalidArguments(String),
    #[error("tool is unavailable: {0}")]
    Unavailable(String),
    #[error("tool invocation failed: {0}")]
    Failed(String),
}

#[async_trait]
pub trait ToolRegistry: Send + Sync {
    async fn definitions(&self) -> Vec<ModelToolDefinition>;
    async fn invoke_read_only(
        &self,
        call_id: &str,
        name: &str,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> Result<ModelToolResult, ToolError>;
}

#[derive(Default)]
pub struct NoTools;

#[async_trait]
impl ToolRegistry for NoTools {
    async fn definitions(&self) -> Vec<ModelToolDefinition> {
        Vec::new()
    }

    async fn invoke_read_only(
        &self,
        _call_id: &str,
        _name: &str,
        _arguments: Value,
        _cancellation: CancellationToken,
    ) -> Result<ModelToolResult, ToolError> {
        Err(ToolError::Unknown)
    }
}

pub struct UnixToolRegistry {
    socket: PathBuf,
    definitions: Vec<ModelToolDefinition>,
    timeout: Duration,
}

impl UnixToolRegistry {
    pub fn new(socket: PathBuf) -> Self {
        Self {
            socket,
            definitions: safe_tool_definitions(),
            timeout: Duration::from_secs(15),
        }
    }
}

#[async_trait]
impl ToolRegistry for UnixToolRegistry {
    async fn definitions(&self) -> Vec<ModelToolDefinition> {
        self.definitions.clone()
    }

    async fn invoke_read_only(
        &self,
        call_id: &str,
        name: &str,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> Result<ModelToolResult, ToolError> {
        let definition = self
            .definitions
            .iter()
            .find(|value| value.name == name)
            .ok_or(ToolError::Unknown)?;
        if definition.class != ToolClass::ReadOnly {
            return Err(ToolError::Unavailable(
                "router exposes only unprivileged read-only tools".to_owned(),
            ));
        }
        verify_tool_socket(&self.socket)
            .map_err(|error| ToolError::Unavailable(error.to_string()))?;
        let stream = UnixStream::connect(&self.socket)
            .await
            .map_err(|error| ToolError::Unavailable(error.to_string()))?;
        let credentials = stream
            .peer_cred()
            .map_err(|error| ToolError::Unavailable(error.to_string()))?;
        if credentials.uid() != unsafe { libc::geteuid() } {
            return Err(ToolError::Unavailable(
                "tool service peer UID did not match router UID".to_owned(),
            ));
        }
        let tool_name = parse_tool_name(name).ok_or(ToolError::Unknown)?;
        let request_id = BoundedString::new(call_id).map_err(|_| {
            ToolError::InvalidArguments("tool call identifier exceeds 64 bytes".to_owned())
        })?;
        let request = ToolRequest {
            version: PROTOCOL_VERSION_V1.to_owned(),
            request_id: request_id.clone(),
            name: tool_name,
            args: arguments,
            max_input_bytes: MAX_TOOL_INPUT_BYTES_V1 as u32,
            provenance: ToolProvenance {
                source: ToolProvenanceSource::Router,
                timestamp_ms: unix_ms(),
                request_id: Some(request_id),
            },
        };
        request
            .validate()
            .map_err(|error| ToolError::InvalidArguments(error.message))?;
        let stream_id = generated_id("tool-stream");
        let frame = JsonlFrame::ToolRequest {
            version: PROTOCOL_VERSION_V1.to_owned(),
            frame_id: generated_id("tool-frame"),
            stream_id: stream_id.clone(),
            tool_request: request,
        };
        let (read, mut write) = stream.into_split();
        let mut bytes =
            serde_json::to_vec(&frame).map_err(|error| ToolError::Failed(error.to_string()))?;
        bytes.push(b'\n');
        write
            .write_all(&bytes)
            .await
            .map_err(|error| ToolError::Unavailable(error.to_string()))?;
        let mut reader = BufReader::new(read);
        let response = tokio::select! {
            _ = cancellation.cancelled() => {
                return Err(ToolError::Failed("tool invocation cancelled".to_owned()));
            }
            response = timeout(self.timeout, read_tool_frame(&mut reader)) => {
                response
                    .map_err(|_| ToolError::Failed("tool invocation timed out".to_owned()))??
            }
        };
        match response {
            JsonlFrame::ModelToolResult {
                stream_id: response_stream,
                tool_result,
                ..
            } => {
                if response_stream != stream_id || tool_result.request_id.as_str() != call_id {
                    return Err(ToolError::Failed(
                        "tool service returned a mismatched request".to_owned(),
                    ));
                }
                tool_result
                    .validate()
                    .map_err(|error| ToolError::Failed(error.message))?;
                if tool_result.name != tool_name {
                    return Err(ToolError::Failed(
                        "tool service returned a mismatched tool name".to_owned(),
                    ));
                }
                match tool_result.status {
                    ModelToolResultStatus::Ok => Ok(ModelToolResult {
                        content: serde_json::to_string(&tool_result.data)
                            .map_err(|error| ToolError::Failed(error.to_string()))?,
                        provenance: format!(
                            "sentia-tools:{}:{}ms",
                            tool_name.as_str(),
                            tool_result.duration_ms
                        ),
                    }),
                    status => Err(ToolError::Failed(format!(
                        "tool service returned {status:?}: {}",
                        tool_result
                            .error
                            .map(|error| error.message)
                            .unwrap_or_else(|| "no detail".to_owned())
                    ))),
                }
            }
            JsonlFrame::Error { error, .. } => Err(ToolError::Failed(error.message)),
            _ => Err(ToolError::Failed(
                "tool service returned an unexpected frame".to_owned(),
            )),
        }
    }
}

pub async fn validate_and_invoke(
    registry: &dyn ToolRegistry,
    definitions: &[ModelToolDefinition],
    call: &BrokerToolCall,
    seen: &mut HashSet<String>,
    cancellation: CancellationToken,
) -> Result<ModelToolResult, ToolError> {
    if !seen.insert(call.id.clone()) {
        return Err(ToolError::InvalidArguments(
            "duplicate tool call identifier".to_owned(),
        ));
    }
    if call.id.is_empty() || call.id.len() > 64 || call.arguments.len() > MAX_TOOL_ARGUMENT_BYTES {
        return Err(ToolError::InvalidArguments(
            "tool call identifier or arguments exceed protocol bounds".to_owned(),
        ));
    }
    let definition = definitions
        .iter()
        .find(|value| value.name == call.name)
        .ok_or(ToolError::Unknown)?;
    if definition.class != ToolClass::ReadOnly {
        return Err(ToolError::Unavailable(
            "action proposals require a separate user approval through the privileged broker"
                .to_owned(),
        ));
    }
    let arguments: Value = serde_json::from_str(&call.arguments)
        .map_err(|error| ToolError::InvalidArguments(error.to_string()))?;
    validate_schema(&definition.input_schema, &arguments)?;
    if arguments
        .get("path")
        .and_then(Value::as_str)
        .map(forbidden_tool_path)
        .unwrap_or(false)
    {
        return Err(ToolError::Unavailable(
            "credential and private-state paths are not available to the model".to_owned(),
        ));
    }
    let result = registry
        .invoke_read_only(&call.id, &call.name, arguments, cancellation)
        .await?;
    if result.content.len() > MAX_TOOL_RESULT_BYTES {
        return Err(ToolError::Failed(
            "tool result exceeds size limit".to_owned(),
        ));
    }

    fn forbidden_tool_path(path: &str) -> bool {
        let normalized = path.to_ascii_lowercase();
        [
            "/.ssh/",
            "/.gnupg/",
            "/.aws/",
            "/.config/gcloud/",
            "/.config/gh/",
            "/.local/share/keyrings/",
            "/keyrings/",
            "/cookies",
            "/login data",
            "/.bash_history",
            "/.zsh_history",
            "/etc/shadow",
            "/etc/ssl/private/",
        ]
        .iter()
        .any(|needle| normalized.contains(needle))
    }
    Ok(result)
}

fn safe_tool_definitions() -> Vec<ModelToolDefinition> {
    let registry = sentia_protocol::ToolRegistry::required_v1();
    registry
        .tools
        .into_iter()
        .filter(|tool| tool.privilege_class == PrivilegeClass::User)
        .filter_map(|tool| {
            let (description, input_schema) = safe_model_schema(tool.name)?;
            Some(ModelToolDefinition {
                name: tool.name.as_str().to_owned(),
                description: description.to_owned(),
                input_schema,
                class: ToolClass::ReadOnly,
                provenance: format!("sentia-tools:{}", tool.name.as_str()),
            })
        })
        .collect()
}

fn safe_model_schema(name: ToolName) -> Option<(&'static str, Value)> {
    use serde_json::json;
    let no_args = || json!({"type":"object","properties":{},"additionalProperties":false});
    let string_arg = |argument: &str| {
        json!({
            "type":"object",
            "properties": {
                (argument): {"type":"string","maxLength":4096}
            },
            "required":[argument],
            "additionalProperties":false
        })
    };
    match name {
        ToolName::CommandExists => Some(("Check whether a command exists", string_arg("command"))),
        ToolName::CommandLookup => Some(("Look up commands by name", string_arg("query"))),
        ToolName::CommandHelp => Some(("Read installed command help", string_arg("command"))),
        ToolName::AptSearch => Some(("Search available packages", string_arg("query"))),
        ToolName::AptPackageInfo => Some(("Read package metadata", string_arg("package"))),
        ToolName::AptPackagePolicy => {
            Some(("Read package installation policy", string_arg("package")))
        }
        ToolName::AptSimulateInstall => Some((
            "Simulate a package installation without changing the system",
            string_arg("package"),
        )),
        ToolName::ServiceList => Some(("List system services", no_args())),
        ToolName::ServiceStatus => Some(("Read service status", string_arg("unit"))),
        ToolName::CpuStatus => Some(("Read CPU status", no_args())),
        ToolName::MemoryStatus => Some(("Read memory status", no_args())),
        ToolName::SwapStatus => Some(("Read swap status", no_args())),
        ToolName::FilesystemStatus => Some(("Read filesystem capacity status", no_args())),
        ToolName::NetworkStatus => Some(("Read local network status", no_args())),
        ToolName::NetworkInterfaces => Some(("List local network interfaces", no_args())),
        ToolName::ListeningPorts => Some(("List listening local ports", no_args())),
        ToolName::ProcessList => Some(("List running processes", no_args())),
        ToolName::FileExists => Some(("Check whether a local path exists", string_arg("path"))),
        ToolName::FileType => Some(("Read a local path type", string_arg("path"))),
        ToolName::FilePermissions => Some(("Read local path permissions", string_arg("path"))),
        ToolName::DirectoryContents => Some((
            "List a local directory without modifying it",
            string_arg("path"),
        )),
        ToolName::DiskUsage => Some(("Read disk usage for a local path", string_arg("path"))),
        ToolName::OsInformation => Some(("Read operating system information", no_args())),
        ToolName::HardwareInformation => Some(("Read bounded hardware information", no_args())),
        _ => None,
    }
}

fn parse_tool_name(name: &str) -> Option<ToolName> {
    serde_json::from_value(Value::String(name.to_owned())).ok()
}

fn verify_tool_socket(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_socket()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "tool socket must be owned by the user and inaccessible to other users",
        ));
    }
    Ok(())
}

async fn read_tool_frame<R>(reader: &mut R) -> Result<JsonlFrame, ToolError>
where
    R: AsyncBufRead + Unpin,
{
    let mut bytes = Vec::new();
    loop {
        let available = reader
            .fill_buf()
            .await
            .map_err(|error| ToolError::Failed(error.to_string()))?;
        if available.is_empty() {
            return Err(ToolError::Failed(
                "tool service closed without a response".to_owned(),
            ));
        }
        let take = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| index + 1)
            .unwrap_or(available.len());
        if bytes.len().saturating_add(take) > MAX_TOOL_RESULT_BYTES + 4096 {
            return Err(ToolError::Failed(
                "tool response exceeded bounds".to_owned(),
            ));
        }
        bytes.extend_from_slice(&available[..take]);
        let complete = available[take - 1] == b'\n';
        reader.consume(take);
        if complete {
            break;
        }
    }
    let frame: JsonlFrame =
        serde_json::from_slice(&bytes).map_err(|error| ToolError::Failed(error.to_string()))?;
    frame
        .validate_version()
        .map_err(|error| ToolError::Failed(error.message))?;
    Ok(frame)
}

static TOOL_SEQUENCE: AtomicU64 = AtomicU64::new(1);

fn generated_id(prefix: &str) -> BoundedString<64> {
    BoundedString::new(format!(
        "{prefix}-{}-{}",
        std::process::id(),
        TOOL_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ))
    .expect("generated tool identifier is within bounds")
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn validate_schema(schema: &Value, value: &Value) -> Result<(), ToolError> {
    let Some(schema) = schema.as_object() else {
        return Err(ToolError::InvalidArguments(
            "tool schema must be an object".to_owned(),
        ));
    };
    let expected = schema.get("type").and_then(Value::as_str).unwrap_or("object");
    if expected != json_type(value) {
        return Err(ToolError::InvalidArguments(format!(
            "expected {expected}, got {}",
            json_type(value)
        )));
    }
    if expected != "object" {
        return Ok(());
    }
    let object = value
        .as_object()
        .ok_or_else(|| ToolError::InvalidArguments("expected object".to_owned()))?;
    validate_object(schema, object)
}

fn validate_object(
    schema: &Map<String, Value>,
    object: &Map<String, Value>,
) -> Result<(), ToolError> {
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let required: HashSet<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    for key in required {
        if !object.contains_key(key) {
            return Err(ToolError::InvalidArguments(format!(
                "missing required property {key}"
            )));
        }
    }
    if schema
        .get("additionalProperties")
        .and_then(Value::as_bool)
        == Some(false)
    {
        for key in object.keys() {
            if !properties.contains_key(key) {
                return Err(ToolError::InvalidArguments(format!(
                    "unexpected property {key}"
                )));
            }
        }
    }
    for (name, value) in object {
        let Some(property) = properties.get(name).and_then(Value::as_object) else {
            continue;
        };
        if let Some(expected) = property.get("type").and_then(Value::as_str) {
            if expected != json_type(value) {
                return Err(ToolError::InvalidArguments(format!(
                    "property {name} must be {expected}"
                )));
            }
        }
        if let Some(maximum) = property.get("maxLength").and_then(Value::as_u64) {
            if value.as_str().map(str::len).unwrap_or(0) > maximum as usize {
                return Err(ToolError::InvalidArguments(format!(
                    "property {name} exceeds maxLength"
                )));
            }
        }
    }
    Ok(())
}

fn json_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

pub fn definition_map(definitions: &[ModelToolDefinition]) -> HashMap<String, ModelToolDefinition> {
    definitions
        .iter()
        .cloned()
        .map(|value| (value.name.clone(), value))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rejects_missing_and_extra_arguments() {
        let schema = json!({
            "type": "object",
            "properties": {"name": {"type": "string", "maxLength": 8}},
            "required": ["name"],
            "additionalProperties": false
        });
        assert!(validate_schema(&schema, &json!({})).is_err());
        assert!(validate_schema(&schema, &json!({"name": "ok", "extra": 1})).is_err());
        assert!(validate_schema(&schema, &json!({"name": "toolongvalue"})).is_err());
        assert!(validate_schema(&schema, &json!({"name": "ok"})).is_ok());
    }

    #[test]
    fn model_registry_excludes_privileged_tools() {
        let definitions = safe_tool_definitions();
        assert!(definitions.iter().all(|definition| {
            !matches!(
                definition.name.as_str(),
                "apt_install"
                    | "apt_remove"
                    | "apt_update"
                    | "apt_upgrade"
                    | "service_start"
                    | "service_stop"
                    | "service_restart"
                    | "service_enable"
                    | "process_kill"
            )
        }));
    }

    #[test]
    fn credential_paths_are_forbidden() {
        assert!(forbidden_tool_path("/home/alice/.ssh/id_ed25519"));
        assert!(forbidden_tool_path("/home/alice/.local/share/keyrings/login.keyring"));
        assert!(!forbidden_tool_path("/home/alice/Documents/report.txt"));
    }
}
