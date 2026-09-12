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

- `/usr/libexec/sentia/sentia-apt`

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
- `native/sentia-apt/protocol/response.schema.json`
- `native/sentia-apt/protocol/broker-contract.json`
- `native/sentia-apt/protocol/commands.json`
