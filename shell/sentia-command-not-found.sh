#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=/dev/null
source "$SCRIPT_DIR/sentia-command-not-found-lib.bash"

if [[ ${1:-} == "--help" || ${1:-} == "-h" ]]; then
  cat <<'USAGE'
Usage: sentia-command-not-found.sh <raw command>

Classifies a missing command using Sentia policy:
  LIKELY_TYPO -> deterministic fuzzy executable match
  MISSING_PACKAGE -> package index lookup
  NATURAL_LANGUAGE -> suggest AI route via `ai`
  UNKNOWN -> no strong hint
USAGE
  exit 0
fi

if [[ $# -eq 0 ]]; then
  echo "expected a command line" >&2
  exit 2
fi

sentia_classify_not_found "$*"
