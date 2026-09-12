#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ALLOWLIST_FILE="${1:-$SCRIPT_DIR/sentia-allowed-binaries.txt}"
OUTPUT_FILE="${2:-$SCRIPT_DIR/50-sentia-origin.pref}"

if [[ ! -f "$ALLOWLIST_FILE" ]]; then
  echo "allowlist file not found: $ALLOWLIST_FILE" >&2
  exit 1
fi

mapfile -t raw_packages < <(grep -Ev '^\s*(#|$)' "$ALLOWLIST_FILE")
if [[ ${#raw_packages[@]} -eq 0 ]]; then
  echo "allowlist is empty: $ALLOWLIST_FILE" >&2
  exit 1
fi

regular_packages=()
base_files_override=false
for pkg in "${raw_packages[@]}"; do
  if [[ "$pkg" == "base-files" ]]; then
    base_files_override=true
  else
    regular_packages+=("$pkg")
  fi
done

if [[ ${#regular_packages[@]} -eq 0 ]]; then
  echo "allowlist must include at least one non-base-files package" >&2
  exit 1
fi

{
  cat <<'PREF'
Package: *
Pin: release o=Sentia
Pin-Priority: -10

PREF

  printf 'Package:'
  for pkg in "${regular_packages[@]}"; do
    printf ' %s' "$pkg"
  done
  printf '\nPin: release o=Sentia\nPin-Priority: 700\n\n'

  if [[ "$base_files_override" == true ]]; then
    cat <<'PREF'
Package: base-files
Pin: release o=Sentia
Pin-Priority: 1001
PREF
  fi
} > "$OUTPUT_FILE"
