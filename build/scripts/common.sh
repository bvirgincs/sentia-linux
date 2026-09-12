#!/usr/bin/env bash
set -euo pipefail

readonly REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly SENTIA_SHARED_ROOT="${SENTIA_SHARED_ROOT:-/home/ubuntu/sentia-linux}"
readonly SENTIA_BUILD_ROOT="${SENTIA_BUILD_ROOT:-${SENTIA_SHARED_ROOT}/.build}"
readonly SENTIA_GLOBAL_ARTIFACTS_DIR="${SENTIA_GLOBAL_ARTIFACTS_DIR:-${SENTIA_SHARED_ROOT}/artifacts}"
readonly SENTIA_BUILDER_ARTIFACTS_DIR="${SENTIA_BUILDER_ARTIFACTS_DIR:-${SENTIA_GLOBAL_ARTIFACTS_DIR}/builder}"
readonly SENTIA_LOCK_FILE="${SENTIA_GLOBAL_ARTIFACTS_DIR}/.locks/heavy.lock"

readonly CHROOT_NAME="debian-trixie-amd64"
readonly CHROOT_DIR="${SENTIA_BUILD_ROOT}/chroots/${CHROOT_NAME}"
readonly CACHE_DIR="${SENTIA_BUILD_ROOT}/cache"
readonly WORK_DIR="${SENTIA_BUILD_ROOT}/work"
readonly LOG_DIR="${SENTIA_BUILDER_ARTIFACTS_DIR}/logs"
readonly MANIFEST_DIR="${SENTIA_BUILDER_ARTIFACTS_DIR}/manifests"
readonly PACKAGE_STAGE_DIR="${SENTIA_BUILDER_ARTIFACTS_DIR}/packages"
readonly REPO_STAGE_DIR="${SENTIA_BUILDER_ARTIFACTS_DIR}/repo"
readonly ISO_STAGE_DIR="${SENTIA_BUILDER_ARTIFACTS_DIR}/iso"
readonly VM_STAGE_DIR="${SENTIA_BUILDER_ARTIFACTS_DIR}/vm"

readonly DEFAULT_PACKAGE_INPUT_DIR="${REPO_ROOT}/build/input/packages/pool"
readonly DEFAULT_SIGNING_INPUT_DIR="${REPO_ROOT}/build/input/signing"
readonly DEFAULT_LIVEBUILD_PACKAGE_LIST_DIR="${REPO_ROOT}/config/live-build/package-lists"
readonly DEFAULT_CALAMARES_DIR="${REPO_ROOT}/config/calamares"

readonly PACKAGE_INPUT_DIR="${SENTIA_PACKAGE_INPUT_DIR:-${DEFAULT_PACKAGE_INPUT_DIR}}"
readonly SIGNING_INPUT_DIR="${SENTIA_SIGNING_INPUT_DIR:-${DEFAULT_SIGNING_INPUT_DIR}}"
readonly LIVEBUILD_PACKAGE_LIST_DIR="${SENTIA_LIVEBUILD_PACKAGE_LIST_DIR:-${DEFAULT_LIVEBUILD_PACKAGE_LIST_DIR}}"
readonly CALAMARES_CONFIG_DIR="${SENTIA_CALAMARES_CONFIG_DIR:-${DEFAULT_CALAMARES_DIR}}"

init_dirs() {
  mkdir -p \
    "${SENTIA_GLOBAL_ARTIFACTS_DIR}/.locks" \
    "${SENTIA_BUILDER_ARTIFACTS_DIR}" \
    "${LOG_DIR}" \
    "${MANIFEST_DIR}" \
    "${PACKAGE_STAGE_DIR}" \
    "${REPO_STAGE_DIR}" \
    "${ISO_STAGE_DIR}" \
    "${VM_STAGE_DIR}" \
    "${CACHE_DIR}" \
    "${WORK_DIR}" \
    "${SENTIA_BUILD_ROOT}/chroots"
}

timestamp_utc() {
  date -u +"%Y%m%dT%H%M%SZ"
}

source_date_epoch() {
  git -C "${REPO_ROOT}" log -1 --format=%ct 2>/dev/null || date -u +%s
}

log() {
  printf '[%s] %s\n' "$(timestamp_utc)" "$*"
}

die() {
  echo "ERROR: $*" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

require_file() {
  [[ -f "$1" ]] || die "required file not found: $1"
}

require_dir_nonempty() {
  local dir="$1"
  [[ -d "$dir" ]] || die "required directory not found: $dir"
  local any_file
  any_file="$(find "$dir" -mindepth 1 -maxdepth 1 -type f | head -n 1 || true)"
  [[ -n "$any_file" ]] || die "required directory is empty: $dir"
}

read_manifest_packages() {
  local manifest_path="$1"
  require_file "$manifest_path"
  grep -Ehv '^[[:space:]]*(#|$)' "$manifest_path"
}

write_atomic() {
  local destination="$1"
  local tmp_path="${destination}.tmp.$$"
  mkdir -p "$(dirname "$destination")"
  cat >"$tmp_path"
  mv "$tmp_path" "$destination"
}

run_heavy() {
  local label="$1"
  shift
  init_dirs
  log "heavy: ${label}"
  flock "${SENTIA_LOCK_FILE}" "$@"
}

run_in_builder() {
  local label="$1"
  local command="$2"

  require_command sudo
  require_command systemd-nspawn
  [[ -d "${CHROOT_DIR}" ]] || die "builder chroot missing: ${CHROOT_DIR}. Run make bootstrap."

  local workspace_host="${SENTIA_BUILDER_WORKSPACE:-${REPO_ROOT}}"
  local workspace_mode="${SENTIA_BUILDER_WORKSPACE_MODE:-ro}"
  [[ -d "${workspace_host}" ]] || die "workspace bind path missing: ${workspace_host}"

  local -a workspace_bind
  if [[ "${workspace_mode}" == "rw" ]]; then
    workspace_bind=(--bind="${workspace_host}:/workspace")
  else
    workspace_bind=(--bind-ro="${workspace_host}:/workspace")
  fi

  run_heavy "${label}" sudo systemd-nspawn \
    --quiet \
    --register=no \
    --directory="${CHROOT_DIR}" \
    "${workspace_bind[@]}" \
    --bind="${SENTIA_BUILDER_ARTIFACTS_DIR}:/artifacts" \
    --bind="${CACHE_DIR}:/var/cache/sentia" \
    --setenv=DEBIAN_FRONTEND=noninteractive \
    --setenv=SOURCE_DATE_EPOCH="$(source_date_epoch)" \
    --setenv=MAKEFLAGS=-j2 \
    /bin/bash -lc "${command}"
}

run_in_builder_no_lock() {
  local label="$1"
  local command="$2"

  require_command sudo
  require_command systemd-nspawn
  [[ -d "${CHROOT_DIR}" ]] || die "builder chroot missing: ${CHROOT_DIR}. Run make bootstrap."

  local workspace_host="${SENTIA_BUILDER_WORKSPACE:-${REPO_ROOT}}"
  local workspace_mode="${SENTIA_BUILDER_WORKSPACE_MODE:-ro}"
  [[ -d "${workspace_host}" ]] || die "workspace bind path missing: ${workspace_host}"

  local -a workspace_bind
  if [[ "${workspace_mode}" == "rw" ]]; then
    workspace_bind=(--bind="${workspace_host}:/workspace")
  else
    workspace_bind=(--bind-ro="${workspace_host}:/workspace")
  fi

  log "builder-no-lock: ${label}"
  sudo systemd-nspawn \
    --quiet \
    --register=no \
    --directory="${CHROOT_DIR}" \
    "${workspace_bind[@]}" \
    --bind="${SENTIA_BUILDER_ARTIFACTS_DIR}:/artifacts" \
    --bind="${CACHE_DIR}:/var/cache/sentia" \
    --setenv=DEBIAN_FRONTEND=noninteractive \
    --setenv=SOURCE_DATE_EPOCH="$(source_date_epoch)" \
    --setenv=MAKEFLAGS=-j2 \
    /bin/bash -lc "${command}"
}

latest_iso_path() {
  local iso
  iso="$(find "${ISO_STAGE_DIR}" -maxdepth 1 -type f -name '*.iso' | sort | tail -n 1 || true)"
  [[ -n "$iso" ]] || return 1
  printf '%s\n' "$iso"
}
