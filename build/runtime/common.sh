#!/usr/bin/env bash
set -euo pipefail

SENTIA_RUNTIME_REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SENTIA_BUILD_ROOT="/home/ubuntu/sentia-linux/.build/runtime"
SENTIA_DOWNLOAD_ROOT="/home/ubuntu/sentia-linux/artifacts/downloads"
SENTIA_HEAVY_LOCK="/home/ubuntu/sentia-linux/artifacts/.locks/heavy.lock"

mkdir -p "${SENTIA_BUILD_ROOT}" "${SENTIA_DOWNLOAD_ROOT}"

log() {
  printf '[%s] %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*" >&2
}

require_tools() {
  local missing=()
  for tool in "$@"; do
    if ! command -v "${tool}" >/dev/null 2>&1; then
      missing+=("${tool}")
    fi
  done
  if ((${#missing[@]} > 0)); then
    printf 'Missing required tools: %s\n' "${missing[*]}" >&2
    return 1
  fi
}

run_with_heavy_lock() {
  local cmd=("$@")
  flock "${SENTIA_HEAVY_LOCK}" "${cmd[@]}"
}
