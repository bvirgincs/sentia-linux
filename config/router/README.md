# Sentia router interfaces

## Per-user API

The router listens on `$XDG_RUNTIME_DIR/sentia/router.sock`. The containing
directory is mode `0700`, the socket is mode `0600`, and both client and server
verify Unix peer credentials.

Every wire message is one newline-terminated
`sentia_protocol::socket::JsonlFrame` with version `sentia.v1`. Chat uses a
`RouterRequest` with capability `inference` and media type
`application/vnd.sentia.chat.v1+json`. Streaming output uses `StreamEvent`;
completion uses `RouterResult`; cancellation uses `CancelRequest` and
`CancelAck`.

Local control operations use an ordinary versioned `RouterRequest` whose
payload media type is `application/vnd.sentia.control.v1+json`. The bounded
payload selects `status`, `settings_get`, `settings_update`, or
`consent_preview`. Results use the corresponding status/settings/consent media
type. Consent tokens bind a provider and SHA-256 of the exact minimized payload,
expire after five minutes, and are consumed once.

## CLI

`ai <question>` streams a human-readable answer. `ai --json <question>` emits
the exact JSONL protocol frames. `ai status`, `ai settings`, settings update
subcommands, stdin input, and interactive `ai ask-remote PROVIDER QUESTION` are
implemented. Ctrl-C sends a protocol cancellation frame instead of abandoning
the provider silently.

The default policy is `LOCAL_ONLY`. Remote-preferred routing is useful only
after a separate provider-worker crate registers an eligible provider and the
user explicitly permits every included privacy category. The router process
itself is restricted to Unix sockets and contains no remote HTTP client.

## Local broker

`sentia-local-broker` accepts only `chat`, `health`, and `cancel` operations on
`/run/sentia-local/broker.sock`. It authenticates peer UIDs, enforces per-UID
and global queue limits, serializes inference by default, and forwards OpenAI
compatible HTTP/SSE directly over `/run/sentia-inference/llama.sock` using
native Rust. It exposes no model-control operation and has no TCP transport.

The systemd unit starts alongside `sentia-local-llama.service` and runs as the
unprivileged `sentia-inference` account so it can reach the contract-mandated
mode-`0600` private inference socket. The public broker socket is mode `0666`,
like the health metrics socket: every local login session must be able to reach
the path, and authorisation is taken from `SO_PEERCRED` rather than from the
filesystem mode. Connections are accepted only from UIDs inside the regular
login range (`minimum_peer_uid`..`maximum_peer_uid`, 1000..60000 by default),
which excludes system and systemd `DynamicUser` accounts, and every connection
is then independently attributed and quota-limited by peer UID.

## Privacy and tools

Unknown context stays local. Remote routing is denied unless the prompt and all
context categories are persistently eligible or an exact one-time consent
token is supplied. Credential-like text is redacted, known credential paths
are forbidden, and prompts are never logged.

The agent loop is bounded to four rounds and fixed output limits. When a
per-user tool socket is configured, it sends the shared `ToolRequest` and
accepts only validated `ToolResult` frames. Only unprivileged read-only tools
are exposed to the model. Root/elevated operations are never invoked; an action
can only be surfaced as a proposal for a separate privileged workflow and user
authorization.
