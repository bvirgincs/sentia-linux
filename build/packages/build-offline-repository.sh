#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=build/packages/lib.sh
source "$SCRIPT_DIR/lib.sh"

BUILD_MODE="${SENTIA_PACKAGE_BUILD_MODE:-binary}"
OUTPUT_DIR="${1:-$SENTIA_REPO_ROOT/artifacts/packages/offline-repository}"
WORK_ROOT="$SENTIA_REPO_ROOT/.build/packages/offline-repository"
MANIFEST="$SENTIA_REPO_ROOT/build/packages/manifests/offline-repository.manifest"
COMPACT_ARCHIVE_DIR="$(realpath -m "${SENTIA_COMPACT_ARCHIVE_DIR:-$SENTIA_REPO_ROOT/artifacts/repository/compact}")"
ALLOWED_ARCHIVE_ROOT="$(realpath -m "$SENTIA_REPO_ROOT/artifacts/repository")"

if [[ ! -d "$COMPACT_ARCHIVE_DIR" ]]; then
  echo "missing compact archive directory: $COMPACT_ARCHIVE_DIR" >&2
  exit 1
fi

case "$COMPACT_ARCHIVE_DIR" in
  "$ALLOWED_ARCHIVE_ROOT"/*) ;;
  *)
    echo "compact archive dir is outside allowed root ($ALLOWED_ARCHIVE_ROOT): $COMPACT_ARCHIVE_DIR" >&2
    exit 1
    ;;
esac

if find "$COMPACT_ARCHIVE_DIR" -type f \( -name '*.gguf' -o -name 'sentia-offline-repository_*.deb' \) | grep -q .; then
  echo "compact archive contains forbidden payloads" >&2
  exit 1
fi

"$SENTIA_REPO_ROOT/build/packages/stage-input-manifest.sh" "$MANIFEST" "$WORK_ROOT"

mkdir -p "$WORK_ROOT/packaging/offline-repository/staged-archive-source"
cp -a "$COMPACT_ARCHIVE_DIR/." "$WORK_ROOT/packaging/offline-repository/staged-archive-source/"

run_dpkg_buildpackage "$WORK_ROOT/packaging/offline-repository" "$BUILD_MODE"
copy_deb_outputs "$WORK_ROOT/packaging" "$OUTPUT_DIR"
