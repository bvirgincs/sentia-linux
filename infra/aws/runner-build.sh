#!/bin/bash
# Sentia build driver executed on the disposable AWS test runner.
set -u
exec > /var/log/sentia-build.log 2>&1
echo "=== START $(date -u +%FT%TZ) ==="
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq && apt-get install -y -qq git make sudo rsync ca-certificates gnupg || exit 10
install -d -o ubuntu -g ubuntu /home/ubuntu/work
cd /home/ubuntu/work || exit 11
if [ -d sentia-linux/.git ]; then
  sudo -u ubuntu git -C sentia-linux fetch --quiet origin main
  sudo -u ubuntu git -C sentia-linux reset --hard --quiet origin/main
else
  sudo -u ubuntu git clone --quiet https://github.com/bvirgincs/sentia-linux.git || exit 12
fi
cd sentia-linux || exit 13
echo "HEAD: $(git rev-parse --short HEAD)"
run_step() {
  local step="$1"; shift
  echo "=== STEP $step $(date -u +%FT%TZ) ==="
  local rc=0
  # Do not wrap this in `if`: a failed `if` condition with no else branch
  # resets $? to 0, which previously reported every failure as rc=0.
  sudo -u ubuntu --preserve-env=DEBIAN_FRONTEND,SENTIA_REPO_SIGNING_KEY_ID "$@" || rc=$?
  if [ "$rc" -eq 0 ]; then
    echo "=== STEP $step OK ==="
    return 0
  fi
  echo "=== STEP $step FAILED rc=$rc ==="
  echo "FAILED_AT=$step" > /var/log/sentia-build.status
  exit $rc
}
run_step bootstrap make bootstrap
FPR_FILE=/home/ubuntu/.local/share/sentia-dev-signing/dev-key.fingerprint
if [ ! -f "$FPR_FILE" ]; then
  echo "missing development signing fingerprint: $FPR_FILE"
  echo "FAILED_AT=signing-key" > /var/log/sentia-build.status
  exit 20
fi
export SENTIA_REPO_SIGNING_KEY_ID="$(cat "$FPR_FILE")"
echo "signing key id: $SENTIA_REPO_SIGNING_KEY_ID"
run_step packages make packages
run_step repo make repo
run_step iso make iso
run_step test-iso make test-iso
echo "ALL_OK" > /var/log/sentia-build.status
echo "=== DONE $(date -u +%FT%TZ) ==="
