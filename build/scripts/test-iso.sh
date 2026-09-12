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

set +e
run_heavy "qemu TCG ISO smoke boot" timeout 180s qemu-system-x86_64 \
  -machine q35,accel=tcg \
  -cpu max \
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

{
  echo "generated_at=$(timestamp_utc)"
  echo "iso_path=${iso_path}"
  echo "qemu_exit_code=${qemu_rc}"
  echo "serial_log=${serial_log}"
} | write_atomic "${MANIFEST_DIR}/test-iso.txt"

log "ISO smoke boot completed with qemu exit code ${qemu_rc}"
log "serial log: ${serial_log}"
