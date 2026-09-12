#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
set -Eeuo pipefail

WORK=/opt/sentia/artifacts/nested-kvm-proof
CACHE=/opt/sentia/cache
IMAGE=noble-server-cloudimg-amd64.img
BASE_URL=https://cloud-images.ubuntu.com/noble/current
QEMU_PID=

cleanup() {
  if [[ -n "${QEMU_PID}" ]] && kill -0 "${QEMU_PID}" 2>/dev/null; then
    kill "${QEMU_PID}"
    wait "${QEMU_PID}" || true
  fi
}
trap cleanup EXIT

test -c /dev/kvm
test -r /dev/kvm
test -w /dev/kvm
grep -Eq '\b(vmx|svm)\b' /proc/cpuinfo
kvm_ok_output=$(kvm-ok 2>&1 || true)
grep -q 'KVM acceleration can be used' <<<"${kvm_ok_output}"

install -d -m 0755 "${WORK}" "${CACHE}"
cd "${CACHE}"
curl --fail --location --retry 3 --silent --show-error \
  --output SHA256SUMS "${BASE_URL}/SHA256SUMS"
curl --fail --location --retry 3 --silent --show-error \
  --output "${IMAGE}" "${BASE_URL}/${IMAGE}"
grep " \\*\\?${IMAGE}$" SHA256SUMS > "${WORK}/image.sha256"
(cd "${CACHE}" && sha256sum --check "${WORK}/image.sha256")

rm -f "${WORK}/guest.qcow2" "${WORK}/seed.img" "${WORK}/OVMF_VARS.fd"
rm -f "${WORK}/qmp.sock" "${WORK}/serial.log" "${WORK}/guest-cpu.json"
rm -f "${WORK}/known_hosts"
ssh-keygen -q -t ed25519 -N '' -f "${WORK}/guest-key"
chmod 0600 "${WORK}/guest-key"
pubkey=$(cat "${WORK}/guest-key.pub")
cat >"${WORK}/user-data" <<EOF
#cloud-config
ssh_pwauth: false
users:
  - default
  - name: sentia-test
    lock_passwd: true
    shell: /bin/bash
    sudo: ALL=(ALL) NOPASSWD:ALL
    ssh_authorized_keys:
      - ${pubkey}
runcmd:
  - [ sh, -c, "install -d /run/sentia-test && touch /run/sentia-test/ready" ]
EOF
cat >"${WORK}/meta-data" <<'EOF'
instance-id: sentia-nested-kvm-proof
local-hostname: sentia-kvm-guest
EOF
cloud-localds "${WORK}/seed.img" "${WORK}/user-data" "${WORK}/meta-data"
qemu-img create -q -f qcow2 -F qcow2 -b "${CACHE}/${IMAGE}" \
  "${WORK}/guest.qcow2" 12G

OVMF_CODE=/usr/share/OVMF/OVMF_CODE_4M.fd
OVMF_VARS=/usr/share/OVMF/OVMF_VARS_4M.fd
test -r "${OVMF_CODE}"
test -r "${OVMF_VARS}"
cp "${OVMF_VARS}" "${WORK}/OVMF_VARS.fd"

qemu-system-x86_64 \
  -name sentia-nested-proof \
  -machine q35,accel=kvm \
  -enable-kvm \
  -cpu host \
  -smp 2 \
  -m 2048 \
  -nodefaults \
  -no-reboot \
  -display none \
  -monitor none \
  -serial "file:${WORK}/serial.log" \
  -qmp "unix:${WORK}/qmp.sock,server=on,wait=off" \
  -drive "if=pflash,format=raw,readonly=on,file=${OVMF_CODE}" \
  -drive "if=pflash,format=raw,file=${WORK}/OVMF_VARS.fd" \
  -drive "if=none,id=root,format=qcow2,cache=none,file=${WORK}/guest.qcow2" \
  -device virtio-blk-pci,drive=root \
  -drive "if=none,id=seed,format=raw,readonly=on,file=${WORK}/seed.img" \
  -device virtio-blk-pci,drive=seed \
  -netdev user,id=net0,hostfwd=tcp:127.0.0.1:2222-:22 \
  -device virtio-net-pci,netdev=net0 &
QEMU_PID=$!

for _ in $(seq 1 120); do
  if ssh -q \
      -i "${WORK}/guest-key" \
      -p 2222 \
      -o BatchMode=yes \
      -o StrictHostKeyChecking=accept-new \
      -o UserKnownHostsFile="${WORK}/known_hosts" \
      -o ConnectTimeout=5 \
      sentia-test@127.0.0.1 \
      'test -f /run/sentia-test/ready && lscpu --json' \
      >"${WORK}/guest-cpu.json"; then
    break
  fi
  if ! kill -0 "${QEMU_PID}" 2>/dev/null; then
    echo "nested guest exited before SSH became responsive" >&2
    exit 1
  fi
  sleep 5
done

test -s "${WORK}/guest-cpu.json"
jq -e '.lscpu[] | select(.field == "Architecture:" and .data == "x86_64")' \
  "${WORK}/guest-cpu.json" >/dev/null
ssh -q \
  -i "${WORK}/guest-key" \
  -p 2222 \
  -o BatchMode=yes \
  -o StrictHostKeyChecking=accept-new \
  -o UserKnownHostsFile="${WORK}/known_hosts" \
  sentia-test@127.0.0.1 \
  'systemctl is-system-running --wait >/dev/null || test "$(systemctl is-system-running)" = degraded'

printf '%s\n' "nested KVM guest and authenticated loopback SSH verified"
