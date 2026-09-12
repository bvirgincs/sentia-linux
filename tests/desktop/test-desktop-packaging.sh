#!/bin/sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)"

control_desktop="$ROOT_DIR/packaging/desktop/debian/control"
control_calamares="$ROOT_DIR/packaging/calamares/debian/control"
control_live="$ROOT_DIR/packaging/live/debian/control"
copyright_desktop="$ROOT_DIR/packaging/desktop/debian/copyright"
copyright_calamares="$ROOT_DIR/packaging/calamares/debian/copyright"
copyright_live="$ROOT_DIR/packaging/live/debian/copyright"

grep -q '^Package: sentia-desktop$' "$control_desktop"
grep -q '^Package: sentia-desktop-settings$' "$control_desktop"
if grep -q 'sentia-ai' "$control_desktop"; then
    echo "sentia-desktop unexpectedly depends on sentia-ai" >&2
    exit 1
fi

grep -q '^Package: sentia-calamares-settings$' "$control_calamares"
grep -q '^Package: sentia-live$' "$control_live"

grep -q ' sentia-archive-keyring,' "$control_calamares"
grep -q ' sentia-repository-config,' "$control_calamares"
if grep -q '^Recommends: sentia-archive-keyring' "$control_calamares"; then
    echo "sentia-archive-keyring must be Depends, not Recommends" >&2
    exit 1
fi

for file in "$control_desktop" "$control_calamares" "$control_live"; do
    grep -q '^Homepage: https://github.com/bvirgincs/sentia-linux$' "$file"
done
for file in "$copyright_desktop" "$copyright_calamares" "$copyright_live"; do
    grep -q '^Source: https://github.com/bvirgincs/sentia-linux$' "$file"
done

if grep -Eqs '\.\./\.\./config' "$ROOT_DIR/packaging/desktop/debian/rules" "$ROOT_DIR/packaging/calamares/debian/rules"; then
    echo "debian/rules must not reference ../../config directly" >&2
    exit 1
fi

grep -q '^sentia-desktop$' "$ROOT_DIR/config/live-build/config/package-lists/sentia-desktop.list.chroot"
grep -q '^sentia-calamares-settings$' "$ROOT_DIR/config/live-build/config/package-lists/sentia-desktop.list.chroot"
grep -q '^sentia-live$' "$ROOT_DIR/config/live-build/config/package-lists/sentia-desktop.list.chroot"

if [ -e "$ROOT_DIR/config/desktop/usr/share/applications/sentia-terminal.desktop" ] || \
   [ -e "$ROOT_DIR/config/desktop/usr/share/applications/sentia-system-monitor.desktop" ] || \
   [ -e "$ROOT_DIR/config/desktop/usr/libexec/sentia-desktop/launch-terminal" ] || \
   [ -e "$ROOT_DIR/config/desktop/usr/libexec/sentia-desktop/launch-monitor" ]; then
    echo "placeholder Sentia app launchers/wrappers must not be shipped by desktop settings" >&2
    exit 1
fi

echo "desktop packaging metadata checks passed"
