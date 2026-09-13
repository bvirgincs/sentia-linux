#!/usr/bin/env bash
set -euo pipefail

SENTIA_RUNTIME_REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SENTIA_BUILD_ROOT="${SENTIA_BUILD_ROOT:-${SENTIA_RUNTIME_REPO_ROOT}/.build/runtime}"
SENTIA_DOWNLOAD_ROOT="${SENTIA_DOWNLOAD_ROOT:-${SENTIA_RUNTIME_REPO_ROOT}/artifacts/downloads}"
SENTIA_HEAVY_LOCK="${SENTIA_HEAVY_LOCK:-${SENTIA_RUNTIME_REPO_ROOT}/artifacts/.locks/heavy.lock}"

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
  if ! flock -w "${SENTIA_HEAVY_LOCK_TIMEOUT:-7200}" "${SENTIA_HEAVY_LOCK}" "${cmd[@]}"; then
    local status=$?
    if (( status == 1 )); then
      echo "timed out waiting for the heavy build lock: ${SENTIA_HEAVY_LOCK}" >&2
    fi
    return "${status}"
  fi
}
