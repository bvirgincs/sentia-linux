#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

init_dirs
clean_log="${LOG_DIR}/clean-$(timestamp_utc).log"
exec > >(tee -a "${clean_log}") 2>&1

log "clean log: ${clean_log}"

run_heavy "clean builder generated outputs" sudo bash -lc "
set -euo pipefail
rm -rf \
  '${WORK_DIR}/live-build' \
  '${CACHE_DIR}/live-build' \
  '${SENTIA_BUILDER_ARTIFACTS_DIR}/iso' \
  '${SENTIA_BUILDER_ARTIFACTS_DIR}/logs' \
  '${SENTIA_BUILDER_ARTIFACTS_DIR}/manifests' \
  '${SENTIA_BUILDER_ARTIFACTS_DIR}/packages' \
  '${SENTIA_BUILDER_ARTIFACTS_DIR}/repo' \
  '${SENTIA_BUILDER_ARTIFACTS_DIR}/test-failure' \
  '${SENTIA_BUILDER_ARTIFACTS_DIR}/vm'
mkdir -p \
  '${ISO_STAGE_DIR}' \
  '${LOG_DIR}' \
  '${MANIFEST_DIR}' \
  '${PACKAGE_STAGE_DIR}' \
  '${REPO_STAGE_DIR}' \
  '${VM_STAGE_DIR}'
"

if [[ "${1:-}" == "--all" ]]; then
  run_heavy "remove bootstrap chroot" sudo rm -rf "${CHROOT_DIR}"
fi

log "clean complete"
