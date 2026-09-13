#!/bin/sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)"
cfg="$ROOT_DIR/config/calamares"

grep -q '^branding: sentia$' "$cfg/calamares/settings.conf"
grep -q 'sentia-cleanup-live' "$cfg/calamares/settings.conf"
grep -q 'unpackfs' "$cfg/calamares/settings.conf"
grep -q 'sources-final' "$cfg/calamares/settings.conf"
if grep -q 'sources-media' "$cfg/calamares/settings.conf"; then
    echo "settings.conf must not use sources-media flow" >&2
    exit 1
fi

# calamares-bootloader-config runs update-grub, which needs /boot/grub from
# grub-install and /etc/default/grub from grubcfg, so it must follow both.
order="$(grep -n -E '^ +- (grubcfg|bootloader|bootloader-config)$' "$cfg/calamares/settings.conf" | sed 's/.*- //' | tr '\n' ' ')"
if [ "$order" != "grubcfg bootloader bootloader-config " ]; then
    echo "unexpected bootloader module order: $order" >&2
    exit 1
fi

grep -q 'requiredPartitionTableType: gpt' "$cfg/calamares/modules/partition.conf"
grep -q 'defaultFileSystemType: "ext4"' "$cfg/calamares/modules/partition.conf"
grep -q 'userSwapChoices:' "$cfg/calamares/modules/partition.conf"
grep -q '  - file' "$cfg/calamares/modules/partition.conf"

grep -q 'efiBootloaderId: "debian"' "$cfg/calamares/modules/bootloader.conf"
grep -q 'efiBootLoader: "sb-shim"' "$cfg/calamares/modules/bootloader.conf"

grep -q 'source: "/run/live/medium/live/filesystem.squashfs"' "$cfg/calamares/modules/unpackfs.conf"
grep -q 'Sentia installation requires UEFI boot mode' "$cfg/helpers/calamares-bootloader-config"
grep -q 'dpkg-query -W shim-signed grub-efi-amd64-signed' "$cfg/helpers/calamares-bootloader-config"

grep -q '/usr/share/keyrings/sentia-archive-keyring.gpg' "$cfg/helpers/calamares-sources-final"
grep -q 'file:/usr/share/sentia/archive' "$cfg/helpers/calamares-sources-final"
grep -q 'Suites: sentia-0.1' "$cfg/helpers/calamares-sources-final"
grep -q 'Enabled: yes' "$cfg/helpers/calamares-sources-final"
grep -q '/etc/apt/sources.list.d/sentia.sources' "$cfg/helpers/calamares-sources-final"
if grep -Eq 'apt-get|apt ' "$cfg/helpers/calamares-bootloader-config" "$cfg/helpers/calamares-sources-final" "$cfg/helpers/calamares-cleanup-live"; then
    echo "offline installer helpers must not run apt-get/apt" >&2
    exit 1
fi
if [ -e "$cfg/helpers/calamares-sources-media" ]; then
    echo "calamares-sources-media helper should not exist" >&2
    exit 1
fi
if [ -e "$cfg/calamares-modules/sources-media/module.desc" ] || [ -e "$cfg/calamares-modules/sources-media-unmount/module.desc" ]; then
    echo "sources-media modules should not exist" >&2
    exit 1
fi
if grep -q 'systemctl enable' "$cfg/helpers/calamares-cleanup-live"; then
    echo "cleanup helper must not force-enable guessed services" >&2
    exit 1
fi
if grep -q 'userdel --remove .*||' "$cfg/helpers/calamares-cleanup-live"; then
    echo "cleanup helper must not swallow userdel errors" >&2
    exit 1
fi

if grep -R "trusted=yes" "$cfg" >/dev/null 2>&1; then
    echo "Found forbidden trusted=yes usage" >&2
    exit 1
fi

if grep -R "^URIs: https://.*sentia" "$cfg/helpers" >/dev/null 2>&1; then
    echo "Found forbidden enabled Sentia HTTPS source" >&2
    exit 1
fi

echo "calamares configuration checks passed"
