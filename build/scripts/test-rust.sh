#!/usr/bin/env bash
# Runs the Sentia Rust workspace test suite.
#
# This is a separate target from the Debian package build on purpose. Package
# builds run as root, and much of this suite asserts that Sentia refuses to
# operate on root-owned sockets and indexes, so running it as root inverts the
# assertions. Refuse to run as root rather than reporting misleading failures.
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

init_dirs
test_log="${LOG_DIR}/test-rust-$(timestamp_utc).log"
exec > >(tee -a "${test_log}") 2>&1

log "rust test log: ${test_log}"

if [[ "$(id -u)" -eq 0 ]]; then
  die "refusing to run the workspace test suite as root; several tests assert that Sentia rejects root-owned sockets and indexes"
fi

require_command cargo

cd "${REPO_ROOT}"
run_heavy "cargo test --workspace" \
  cargo test --release --locked --workspace --jobs 2 --no-fail-fast

log "workspace tests passed"
