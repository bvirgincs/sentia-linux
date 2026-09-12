#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
CONFIG_DIR="$REPO_ROOT/config/apt"
WORK_DIR="$REPO_ROOT/.build/tests/packaging/test-config"

rm -rf "$WORK_DIR"
mkdir -p "$WORK_DIR"

"$CONFIG_DIR/generate-sentia-origin-pref.sh" "$CONFIG_DIR/sentia-allowed-binaries.txt" "$WORK_DIR/generated.pref"
cmp -s "$WORK_DIR/generated.pref" "$CONFIG_DIR/50-sentia-origin.pref"

grep -Fq 'Suites: trixie trixie-updates' "$CONFIG_DIR/debian.sources"
grep -Fq 'Suites: trixie-security' "$CONFIG_DIR/debian.sources"
grep -Fq 'Components: main non-free-firmware' "$CONFIG_DIR/debian.sources"
! grep -Eq '\$\{distro\}|\$\{codename\}|\bstable\b|backports|contrib' "$CONFIG_DIR/debian.sources"

grep -Fq 'URIs: file:/usr/share/sentia/archive' "$CONFIG_DIR/sentia.sources"
grep -Fq 'Suites: sentia-0.1' "$CONFIG_DIR/sentia.sources"
grep -Fq 'Signed-By: /usr/share/keyrings/sentia-archive-keyring.gpg' "$CONFIG_DIR/sentia.sources"
! grep -Eq '^URIs: https?://' "$CONFIG_DIR/sentia.sources"

grep -Fq 'origin=Debian,label=Debian-Security,codename=trixie-security' "$CONFIG_DIR/52sentia-unattended-upgrades"
! grep -Eq '\$\{distro\}|\$\{codename\}' "$CONFIG_DIR/52sentia-unattended-upgrades"

grep -Fq 'Package: *' "$CONFIG_DIR/50-sentia-origin.pref"
grep -Fq 'Pin: release o=Sentia' "$CONFIG_DIR/50-sentia-origin.pref"
grep -Fq 'Pin-Priority: -10' "$CONFIG_DIR/50-sentia-origin.pref"
grep -Fq 'Package: base-files' "$CONFIG_DIR/50-sentia-origin.pref"
grep -Fq 'Pin-Priority: 1001' "$CONFIG_DIR/50-sentia-origin.pref"
