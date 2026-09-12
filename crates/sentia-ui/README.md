# sentia-ui

Native Rust GTK4/VTE applications owned by the `work/applications` scope.

## Binaries

- `sentia-terminal`: Bash-first terminal with optional command-not-found help.
- `sentia-monitor`: capability-aware health UI using `/run/sentia-health/metrics.sock` with local fallback.
- `sentia-firstboot`: offline skippable setup wizard (defaults: `LOCAL_ONLY`, telemetry off).

### CLI/API surface (for cross-agent integration)

`sentia-terminal` flags:
- `--assist-mode {command-not-found|on-request|auto-suggest|disabled|conventional}`
- `--conventional` (disables Sentia assistance/capture hooks)
- `--capture`, `--capture-max-entries`
- `--cwd`, `--shell`, `--command`, `--quit-after-ms`

`sentia-monitor` flags:
- `--socket` (defaults to `/run/sentia-health/metrics.sock`)
- `--refresh-ms`, `--quit-after-ms`

`sentia-firstboot` flags:
- `--config`, `--quit-after-ms`

Shell callable:
- `shell/sentia-command-not-found.sh <raw command>`
  - emits `CLASS`, `COMMAND`, optional `SUGGESTION`, `PACKAGES`, `AI_HINT`
  - classes: `LIKELY_TYPO`, `MISSING_PACKAGE`, `NATURAL_LANGUAGE`, `UNKNOWN`

Transport constants currently used by UI:
- router socket: `$XDG_RUNTIME_DIR/sentia/router.sock` (`SENTIA_ROUTER_SOCKET` override)
- health socket: `/run/sentia-health/metrics.sock` (`SENTIA_HEALTH_SOCKET` override)

Note: protocol/core envelopes are provisional in this crate and should be
replaced by shared contract/client crates owned by the contracts/router agents.

## Build and test

```bash
cargo test --manifest-path crates/sentia-ui/Cargo.toml --no-default-features
tests/shell/run.sh
tests/ui/test-launch-xvfb.sh
```
