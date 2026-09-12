# sentia-apt-worker

Structured APT worker implemented in C++ on top of `libapt-pkg`.

## Scope

- Reads typed JSON requests from `stdin` and writes typed JSON responses to `stdout`.
- Supports:
  - `apt_search`
  - `apt_package_info`
  - `apt_package_policy`
  - `apt_simulate_install`
  - `apt_install` (`mode: plan|execute`)
  - `apt_remove` (`mode: plan|execute`)
  - `apt_update` (`mode: plan|execute`)
  - `apt_upgrade` (`mode: plan|execute`)
  - `package_owns_file`
  - `diagnostics`
- Uses `pkgPackageManager` transaction execution (no shell front-end invocation).
- Re-resolves and re-hashes canonical plan state before execute.
- Rejects unapproved source changes, held-package changes, and essential removals by default.

## Installed binary path

The broker-invoked binary path is fixed:

- `/usr/libexec/sentia/sentia-apt-worker`

Compatibility path is also installed as a symlink:

- `/usr/libexec/sentia/sentia-apt` -> `sentia-apt-worker`

## Build

```bash
cmake -S native/sentia-apt -B native/sentia-apt/out
cmake --build native/sentia-apt/out -j2
```

Required Debian build packages:

- `libapt-pkg-dev`
- `nlohmann-json3-dev`
- `cmake`
- `pkg-config`
- `build-essential`

## Protocol

The request/response schema and privilege-broker plan/execute contract live under:

- `native/sentia-apt/protocol/request.schema.json`
- `native/sentia-apt/protocol/request-legacy.schema.json`
- `native/sentia-apt/protocol/response.schema.json`
- `native/sentia-apt/protocol/response-legacy.schema.json`
- `native/sentia-apt/protocol/broker-contract.json`
- `native/sentia-apt/protocol/commands.json`

Shared v1 contract dependency:

- `work/contracts` crate `crates/sentia-protocol`
- schema IDs rooted at `https://sentia.local/schemas/...`

`request.schema.json` and `response.schema.json` are interop wrappers that
reference shared `tool-invocation-v1` request/result definitions and keep
legacy envelopes for broker compatibility during transition.

## Command-not-found lookup contract (read-only)

Use `operation=package_owns_file` for authoritative offline command-to-package
lookup (no automatic execution).

Request (stdin JSON):

```json
{
  "request_id": "cnf-lookup-001",
  "protocol_version": "1.0",
  "operation": "package_owns_file",
  "arguments": {
    "file_path": "docker",
    "index_path": "/usr/share/sentia/command-index/command-index.json"
  }
}
```

Response result fields:
- `file_path`, `command`, `found`
- `packages[]`, `paths[]`
- `index_path`, `index_generated_at`
- `provenance` (suite/snapshot/source_uri/contents hashes)

Interactive guidance:
- client timeout: 2000ms
- on timeout/error: return no package suggestion and continue deterministic
  fallback flow (never auto-install).
