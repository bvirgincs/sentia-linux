#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=build/packages/lib.sh
source "$SCRIPT_DIR/lib.sh"

BUILD_MODE="${SENTIA_PACKAGE_BUILD_MODE:-binary}"
OUTPUT_DIR="${1:-$SENTIA_REPO_ROOT/artifacts/packages/sentia-core}"
WORK_ROOT="$SENTIA_REPO_ROOT/.build/packages/sentia-core"
MANIFEST="$SENTIA_REPO_ROOT/build/packages/manifests/sentia-core.manifest"

"$SENTIA_REPO_ROOT/build/packages/stage-input-manifest.sh" "$MANIFEST" "$WORK_ROOT"

mkdir -p "$WORK_ROOT/packaging/sentia/core/staged-config-inputs"
for file in debian.sources sentia.sources 20auto-upgrades 52sentia-unattended-upgrades 50-sentia-origin.pref; do
  cp -a "$WORK_ROOT/config/apt/$file" "$WORK_ROOT/packaging/sentia/core/staged-config-inputs/$file"
done

acquire_heavy_lock
run_dpkg_buildpackage "$WORK_ROOT/packaging/sentia/core" "$BUILD_MODE"
copy_deb_outputs "$WORK_ROOT/packaging/sentia" "$OUTPUT_DIR"
