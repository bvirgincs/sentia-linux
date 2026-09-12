#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

manifest_dir="$REPO_ROOT/build/packages/manifests"

for manifest in "$manifest_dir"/*.manifest; do
  while IFS= read -r line || [[ -n "$line" ]]; do
    entry="${line%%#*}"
    entry="$(echo "$entry" | sed -E 's/^[[:space:]]+|[[:space:]]+$//g')"
    [[ -z "$entry" ]] && continue

    [[ "$entry" != /* ]]
    [[ "$entry" != *".."* ]]

    case "$entry" in
      packaging/sentia/core*|packaging/base-files*|packaging/keyring*|packaging/offline-repository*|config/apt/*)
        ;;
      *)
        echo "manifest entry escapes owned paths: $entry" >&2
        exit 1
        ;;
    esac

    [[ -e "$REPO_ROOT/$entry" ]] || {
      echo "manifest entry missing from repo: $entry" >&2
      exit 1
    }
  done < "$manifest"
done
