#!/usr/bin/env bash
# Builds the Sentia desktop-stack packages that the live image installs:
# sentia-desktop, sentia-desktop-settings, sentia-calamares-settings and
# sentia-live.
#
# The desktop and calamares source packages expect their payload flattened
# into debian/staged-config, so the manifest-staged config/<name>/ tree is
# moved into place before dpkg-buildpackage runs.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=build/packages/lib.sh
source "$SCRIPT_DIR/lib.sh"

BUILD_MODE="${SENTIA_PACKAGE_BUILD_MODE:-binary}"
OUTPUT_DIR="${1:-$SENTIA_REPO_ROOT/artifacts/packages/desktop-stack}"

build_staged_package() {
  local name="$1"
  local config_subdir="$2"

  local work_root="$SENTIA_REPO_ROOT/.build/packages/$name"
  local manifest="$SENTIA_REPO_ROOT/build/packages/manifests/$name.manifest"

  "$SENTIA_REPO_ROOT/build/packages/stage-input-manifest.sh" "$manifest" "$work_root"

  local source_dir="$work_root/packaging/$name"
  [[ -d "$source_dir" ]] || {
    echo "staged source package missing: $source_dir" >&2
    exit 1
  }

  if [[ -n "$config_subdir" ]]; then
    local staged_config="$source_dir/debian/staged-config"
    rm -rf "$staged_config"
    mkdir -p "$staged_config"
    cp -a "$work_root/$config_subdir/." "$staged_config/"
  fi

  acquire_heavy_lock
  run_dpkg_buildpackage "$source_dir" "$BUILD_MODE"
  copy_deb_outputs "$work_root/packaging" "$OUTPUT_DIR"
}

build_staged_package desktop config/desktop
build_staged_package calamares config/calamares
build_staged_package live ""
