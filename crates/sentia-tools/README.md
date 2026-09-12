# sentia-tools

Deterministic, read-only Linux tooling primitives for Sentia router integration.

Implemented APIs:
- Commands: `command_exists`, `command_lookup`, `command_help`, `command_history`
- Packages: `package_owns_file`
- Services: `service_list`, `service_status`
- Logs: `journal_recent`, `journal_errors`, `kernel_logs`
- Network/processes: `network_status`, `network_interfaces`, `listening_ports`, `process_list`, `process_details`
- Files/system: `file_exists`, `file_type`, `file_permissions`, `directory_contents`, `disk_usage`, `os_information`, `hardware_information`

Non-goals in this crate:
- APT mutation/search flows
- privileged mutation actions
- health metrics (CPU/memory/swap/disk/temp)

Security posture:
- fixed absolute executables only
- validated arguments, no shell evaluation
- sanitized subprocess environment
- timeouts and output limits
- explicit permission-denied/unavailable errors
- command history is explicit-input only (never reads shell history files)
- privileged broker methods (`org.sentia.System1` Prepare/Apply) are explicitly denied in this read-only registry

Router JSON framing (`sentia.tools.readonly.v1`):
- `list`: returns read-only tool descriptors and JSON input schemas
- `invoke`: dispatches by tool name with validated typed JSON input
- `cancel`: explicit form, currently returns `accepted=false` (sync calls only)
- `error`: structured `ToolErrorEnvelope { category, retryable, error }`
