#!/bin/sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)"

cleanup() {
    rm -rf \
      "$ROOT_DIR/packaging/desktop/debian/staged-config" \
      "$ROOT_DIR/packaging/calamares/debian/staged-config"
}

trap cleanup EXIT

"$ROOT_DIR/tests/desktop/stage-package-inputs.sh"

required_desktop="
$ROOT_DIR/packaging/desktop/debian/staged-config/usr/share/backgrounds/sentia/sentia-wallpaper.svg
$ROOT_DIR/packaging/desktop/debian/staged-config/etc/xdg/xfce4/xfconf/xfce-perchannel-xml/xfce4-desktop.xml
"

required_calamares="
$ROOT_DIR/packaging/calamares/debian/staged-config/calamares/settings.conf
$ROOT_DIR/packaging/calamares/debian/staged-config/calamares/modules/partition.conf
$ROOT_DIR/packaging/calamares/debian/staged-config/calamares-modules/sources-final/module.desc
$ROOT_DIR/packaging/calamares/debian/staged-config/helpers/calamares-sources-final
$ROOT_DIR/packaging/calamares/debian/staged-config/COPYING
"

for file in $required_desktop $required_calamares; do
    if [ ! -f "$file" ]; then
        echo "missing staged input: $file" >&2
        exit 1
    fi
done

echo "staging contract checks passed"
