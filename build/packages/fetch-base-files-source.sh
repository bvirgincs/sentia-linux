#!/usr/bin/env bash
set -euo pipefail

VERSION="${1:-13.8+deb13u7}"
DEST_ROOT="${2:-$(pwd)/.build/packages/base-files/upstream}"

mkdir -p "$DEST_ROOT"
archive_path="$DEST_ROOT/base-files_${VERSION}.tar.xz"
source_dir="$DEST_ROOT/base-files-${VERSION}"

if [[ ! -f "$archive_path" ]]; then
  curl --fail --location --silent --show-error \
    "https://deb.debian.org/debian/pool/main/b/base-files/base-files_${VERSION}.tar.xz" \
    --output "$archive_path"
fi

rm -rf "$source_dir"
mkdir -p "$source_dir"
tar -xJf "$archive_path" -C "$source_dir" --strip-components=1

printf '%s\n' "$source_dir"
