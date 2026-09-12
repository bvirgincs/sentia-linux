#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: export-public-keyring.sh <fingerprint> <output-file>" >&2
  exit 1
fi

fingerprint="$1"
output_file="$2"
signing_home="${SENTIA_SIGNING_HOME:-$HOME/.local/share/sentia-dev-signing}"
export GNUPGHOME="$signing_home/gnupg"

if ! gpg --batch --list-keys "$fingerprint" >/dev/null 2>&1; then
  echo "key not found in signing home: $fingerprint" >&2
  exit 1
fi

mkdir -p "$(dirname "$output_file")"
gpg --batch --yes --export "$fingerprint" > "$output_file"
chmod 0644 "$output_file"
