#!/usr/bin/env bash
set -euo pipefail

manifest="/workspace/build/manifests/builder-dependencies.txt"
if [[ ! -f "$manifest" ]]; then
  echo "ERROR: builder dependency manifest is missing: $manifest" >&2
  exit 1
fi

mapfile -t packages < <(grep -Ehv '^[[:space:]]*(#|$)' "$manifest")
if [[ "${#packages[@]}" -eq 0 ]]; then
  echo "ERROR: builder dependency manifest has no packages: $manifest" >&2
  exit 1
fi

apt-get update
apt-get install -y --no-install-recommends "${packages[@]}"
apt-get clean
