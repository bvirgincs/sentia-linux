#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

init_dirs
test_install_log="${LOG_DIR}/test-install-$(timestamp_utc).log"
exec > >(tee -a "${test_install_log}") 2>&1

log "test-install log: ${test_install_log}"

iso_path="$(latest_iso_path || true)"
[[ -n "${iso_path}" ]] || die "no ISO artifact found in ${ISO_STAGE_DIR}; run make iso first"

automation_harness="${REPO_ROOT}/tests/vm/test-install.sh"
if [[ ! -x "${automation_harness}" ]]; then
  die "missing installer automation harness: ${automation_harness} (owned by VM/installer test agent)"
fi

run_heavy "installer acceptance automation" "${automation_harness}" "${iso_path}" "${VM_STAGE_DIR}"

{
  echo "generated_at=$(timestamp_utc)"
  echo "iso_path=${iso_path}"
  echo "harness=${automation_harness}"
} | write_atomic "${MANIFEST_DIR}/test-install.txt"

log "installer acceptance test completed"
