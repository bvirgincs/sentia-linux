#!/usr/bin/env bash
set -euo pipefail
shopt -s extglob

if [[ $# -ne 2 ]]; then
  echo "usage: stage-input-manifest.sh <manifest-file> <staging-root>" >&2
  exit 1
fi

MANIFEST_FILE="$1"
STAGING_ROOT="$2"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

if [[ ! -f "$MANIFEST_FILE" ]]; then
  echo "manifest file not found: $MANIFEST_FILE" >&2
  exit 1
fi

rm -rf "$STAGING_ROOT"
mkdir -p "$STAGING_ROOT"

while IFS= read -r line || [[ -n "$line" ]]; do
  entry="${line%%#*}"
  entry="${entry##+([[:space:]])}"
  entry="${entry%%+([[:space:]])}"
  [[ -z "$entry" ]] && continue

  if [[ "$entry" == /* ]]; then
    echo "manifest entry must be relative: $entry" >&2
    exit 1
  fi
  if [[ "$entry" == *".."* ]]; then
    echo "manifest entry cannot contain '..': $entry" >&2
    exit 1
  fi

  src="$REPO_ROOT/$entry"
  dst="$STAGING_ROOT/$entry"

  if [[ ! -e "$src" ]]; then
    echo "manifest entry missing: $entry" >&2
    exit 1
  fi

  if [[ -d "$src" ]]; then
    mkdir -p "$dst"
    cp -a "$src/." "$dst/"
  else
    mkdir -p "$(dirname "$dst")"
    cp -a "$src" "$dst"
  fi
done < "$MANIFEST_FILE"
