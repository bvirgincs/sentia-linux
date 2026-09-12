#!/bin/sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)"
cfg="$ROOT_DIR/config/calamares"

grep -q '^branding: sentia$' "$cfg/calamares/settings.conf"
grep -q 'sentia-cleanup-live' "$cfg/calamares/settings.conf"
grep -q 'unpackfs' "$cfg/calamares/settings.conf"
grep -q 'sources-final' "$cfg/calamares/settings.conf"

grep -q 'requiredPartitionTableType: gpt' "$cfg/calamares/modules/partition.conf"
grep -q 'defaultFileSystemType: "ext4"' "$cfg/calamares/modules/partition.conf"
grep -q 'userSwapChoices:' "$cfg/calamares/modules/partition.conf"
grep -q '  - file' "$cfg/calamares/modules/partition.conf"

grep -q 'efiBootloaderId: "debian"' "$cfg/calamares/modules/bootloader.conf"
grep -q 'efiBootLoader: "sb-shim"' "$cfg/calamares/modules/bootloader.conf"

grep -q 'source: "/run/live/medium/live/filesystem.squashfs"' "$cfg/calamares/modules/unpackfs.conf"

grep -q '/usr/share/keyrings/sentia-archive-keyring.gpg' "$cfg/helpers/calamares-sources-media"
grep -q 'file:/usr/share/sentia/archive' "$cfg/helpers/calamares-sources-final"
grep -q 'Suites: sentia-0.1' "$cfg/helpers/calamares-sources-final"

if grep -R "trusted=yes" "$cfg" >/dev/null 2>&1; then
    echo "Found forbidden trusted=yes usage" >&2
    exit 1
fi

if grep -R "^URIs: https://.*sentia" "$cfg/helpers" >/dev/null 2>&1; then
    echo "Found forbidden enabled Sentia HTTPS source" >&2
    exit 1
fi

echo "calamares configuration checks passed"
