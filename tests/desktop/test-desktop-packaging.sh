#!/bin/sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)"

control_desktop="$ROOT_DIR/packaging/desktop/debian/control"
control_calamares="$ROOT_DIR/packaging/calamares/debian/control"
control_live="$ROOT_DIR/packaging/live/debian/control"

grep -q '^Package: sentia-desktop$' "$control_desktop"
grep -q '^Package: sentia-desktop-settings$' "$control_desktop"
if grep -q 'sentia-ai' "$control_desktop"; then
    echo "sentia-desktop unexpectedly depends on sentia-ai" >&2
    exit 1
fi

grep -q '^Package: sentia-calamares-settings$' "$control_calamares"
grep -q '^Package: sentia-live$' "$control_live"

grep -q '^sentia-desktop$' "$ROOT_DIR/config/live-build/package-lists/sentia-desktop.list.chroot"
grep -q '^sentia-calamares-settings$' "$ROOT_DIR/config/live-build/package-lists/sentia-desktop.list.chroot"
grep -q '^sentia-live$' "$ROOT_DIR/config/live-build/package-lists/sentia-desktop.list.chroot"

echo "desktop packaging metadata checks passed"
