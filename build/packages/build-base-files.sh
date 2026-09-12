#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=build/packages/lib.sh
source "$SCRIPT_DIR/lib.sh"

BUILD_MODE="${SENTIA_PACKAGE_BUILD_MODE:-binary}"
OUTPUT_DIR="${1:-$SENTIA_REPO_ROOT/artifacts/packages/base-files}"
WORK_ROOT="$SENTIA_REPO_ROOT/.build/packages/base-files"
MANIFEST="$SENTIA_REPO_ROOT/build/packages/manifests/base-files.manifest"
UPSTREAM_VERSION="${SENTIA_BASE_FILES_VERSION:-13.8+deb13u7}"

"$SENTIA_REPO_ROOT/build/packages/stage-input-manifest.sh" "$MANIFEST" "$WORK_ROOT"

if [[ -n "${SENTIA_BASE_FILES_UPSTREAM_DIR:-}" ]]; then
  upstream_source="$(realpath -m "$SENTIA_BASE_FILES_UPSTREAM_DIR")"
else
  upstream_source="$("$SENTIA_REPO_ROOT/build/packages/fetch-base-files-source.sh" "$UPSTREAM_VERSION" "$WORK_ROOT/upstream")"
fi

if [[ ! -d "$upstream_source" ]]; then
  echo "upstream base-files source not found: $upstream_source" >&2
  exit 1
fi

STAGED_SOURCE="$WORK_ROOT/src/base-files"
rm -rf "$STAGED_SOURCE"
mkdir -p "$WORK_ROOT/src"
cp -a "$upstream_source" "$STAGED_SOURCE"

python3 "$WORK_ROOT/packaging/base-files/apply-sentia-delta.py" "$STAGED_SOURCE"

acquire_heavy_lock
run_dpkg_buildpackage "$STAGED_SOURCE" "$BUILD_MODE"
copy_deb_outputs "$WORK_ROOT/src" "$OUTPUT_DIR"
