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

ovmf_code="/usr/share/OVMF/OVMF_CODE.fd"
ovmf_vars_template="/usr/share/OVMF/OVMF_VARS.fd"
require_file "${ovmf_code}"
require_file "${ovmf_vars_template}"

vm_run_dir="${VM_STAGE_DIR}/iso-smoke"
mkdir -p "${vm_run_dir}"
cp -f "${ovmf_vars_template}" "${vm_run_dir}/OVMF_VARS.fd"

if [[ ! -f "${vm_run_dir}/smoke.qcow2" ]]; then
  qemu-img create -f qcow2 "${vm_run_dir}/smoke.qcow2" 24G >/dev/null
fi

serial_log="${vm_run_dir}/serial.log"
rm -f "${serial_log}"

# KVM where the host provides it; TCG is a correctness fallback, not a
# performance-representative run.
if [[ -r /dev/kvm && -w /dev/kvm ]]; then
  accel="kvm"
  cpu_model="host"
  boot_timeout="${SENTIA_ISO_BOOT_TIMEOUT:-180}"
else
  accel="tcg"
  cpu_model="max"
  boot_timeout="${SENTIA_ISO_BOOT_TIMEOUT:-900}"
fi
log "smoke boot acceleration: ${accel}"

set +e
run_heavy "qemu ISO smoke boot (${accel})" timeout "${boot_timeout}s" qemu-system-x86_64 \
  -machine "q35,accel=${accel}" \
  -cpu "${cpu_model}" \
  -smp 2 \
  -m 4096 \
  -name sentia-iso-smoke \
  -display none \
  -monitor none \
  -serial "file:${serial_log}" \
  -no-reboot \
  -drive if=pflash,format=raw,readonly=on,file="${ovmf_code}" \
  -drive if=pflash,format=raw,file="${vm_run_dir}/OVMF_VARS.fd" \
  -drive if=virtio,format=qcow2,file="${vm_run_dir}/smoke.qcow2" \
  -cdrom "${iso_path}"
qemu_rc=$?
set -e

if [[ "${qemu_rc}" -ne 0 && "${qemu_rc}" -ne 124 ]]; then
  die "QEMU smoke boot failed with exit code ${qemu_rc}; see ${serial_log}"
fi

# A timeout on its own proves nothing: the guest could have panicked or sat at
# the firmware. Require evidence from the serial console that the live system
# actually reached the graphical target.
require_file "${serial_log}"
declare -A boot_evidence=(
  ["kernel reached userspace"]="systemd\[1\]"
  ["live filesystem mounted"]="Reached target"
  ["display manager started"]="[Ll]ight[Dd][Mm]"
  ["graphical target reached"]="Reached target Graphical Interface|Reached target graphical.target"
)
missing_evidence=()
for evidence in "${!boot_evidence[@]}"; do
  grep -Eq "${boot_evidence[${evidence}]}" "${serial_log}" || missing_evidence+=("${evidence}")
done
if [[ "${#missing_evidence[@]}" -gt 0 ]]; then
  echo "--- last 60 serial lines ---" >&2
  tail -n 60 "${serial_log}" >&2
  die "ISO boot evidence missing: ${missing_evidence[*]}; see ${serial_log}"
fi
log "ISO boot evidence found for all ${#boot_evidence[@]} checks"

{
  echo "generated_at=$(timestamp_utc)"
  echo "iso_path=${iso_path}"
  echo "qemu_exit_code=${qemu_rc}"
  echo "accel=${accel}"
  echo "serial_log=${serial_log}"
} | write_atomic "${MANIFEST_DIR}/test-iso.txt"

log "ISO smoke boot completed with qemu exit code ${qemu_rc}"
log "serial log: ${serial_log}"
