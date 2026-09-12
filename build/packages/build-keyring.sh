#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=build/packages/lib.sh
source "$SCRIPT_DIR/lib.sh"

BUILD_MODE="${SENTIA_PACKAGE_BUILD_MODE:-binary}"
OUTPUT_DIR="${1:-$SENTIA_REPO_ROOT/artifacts/packages/keyring}"
WORK_ROOT="$SENTIA_REPO_ROOT/.build/packages/keyring"
MANIFEST="$SENTIA_REPO_ROOT/build/packages/manifests/keyring.manifest"
SIGNING_HOME="$(realpath -m "${SENTIA_SIGNING_HOME:-$HOME/.local/share/sentia-dev-signing}")"
KEYRING_INPUT="$(realpath -m "${SENTIA_ARCHIVE_PUBLIC_KEYRING:-$SIGNING_HOME/public/sentia-archive-keyring.gpg}")"

if [[ ! -f "$KEYRING_INPUT" ]]; then
  echo "missing archive keyring input: $KEYRING_INPUT" >&2
  exit 1
fi

case "$KEYRING_INPUT" in
  "$SIGNING_HOME"/*) ;;
  *)
    echo "keyring input is outside allowed signing root ($SIGNING_HOME): $KEYRING_INPUT" >&2
    exit 1
    ;;
esac

"$SENTIA_REPO_ROOT/build/packages/stage-input-manifest.sh" "$MANIFEST" "$WORK_ROOT"

mkdir -p "$WORK_ROOT/packaging/keyring/staged-inputs-source"
cp -a "$KEYRING_INPUT" "$WORK_ROOT/packaging/keyring/staged-inputs-source/sentia-archive-keyring.gpg"

acquire_heavy_lock
run_dpkg_buildpackage "$WORK_ROOT/packaging/keyring" "$BUILD_MODE"
copy_deb_outputs "$WORK_ROOT/packaging" "$OUTPUT_DIR"
