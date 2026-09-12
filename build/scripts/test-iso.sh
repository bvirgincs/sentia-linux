#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

init_dirs
test_iso_log="${LOG_DIR}/test-iso-$(timestamp_utc).log"
exec > >(tee -a "${test_iso_log}") 2>&1

log "test-iso log: ${test_iso_log}"

require_command qemu-system-x86_64
require_command xorriso

iso_path="$(latest_iso_path || true)"
[[ -n "${iso_path}" ]] || die "no ISO artifact found in ${ISO_STAGE_DIR}; run make iso first"

xorriso -indev "${iso_path}" -pvd_info >/dev/null

# Debian and Ubuntu ship the 4 MB firmware split under different names; the
# harness resolves the same candidates.
first_readable_file() {
  local candidate
  for candidate in "$@"; do
    if [[ -r "${candidate}" ]]; then
      printf '%s' "${candidate}"
      return 0
    fi
  done
  return 1
}

# The guest itself is the only reliable witness. tests/vm/live_probe.py boots the
# ISO, logs in over the serial console and runs the acceptance checks inside the
# running live system, so a missing console message can no longer be mistaken
# for a failed boot, or a silent boot for a successful one.
require_command python3

vm_run_dir="${VM_STAGE_DIR}/iso-smoke"
probe_report="${vm_run_dir}/live-probe.json"

probe_args=(
  "${REPO_ROOT}/tests/vm/live_probe.py"
  --iso "${iso_path}"
  --run-dir "${vm_run_dir}"
)
if [[ "${SENTIA_ISO_PROBE_OFFLINE:-0}" == "1" ]]; then
  probe_args+=(--offline)
fi

run_heavy "live ISO acceptance probe" python3 "${probe_args[@]}"
require_file "${probe_report}"
serial_log="${vm_run_dir}/serial.log"

{
  echo "generated_at=$(timestamp_utc)"
  echo "iso_path=${iso_path}"
  echo "probe_report=${probe_report}"
  echo "serial_log=${serial_log}"
  echo "checks_passed=$(python3 -c 'import json,sys; print(len(json.load(open(sys.argv[1]))["checks"]))' "${probe_report}")"
} | write_atomic "${MANIFEST_DIR}/test-iso.txt"

log "live ISO acceptance probe passed"
log "probe report: ${probe_report}"
log "serial log: ${serial_log}"
