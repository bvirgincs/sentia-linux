#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
CLASSIFIER="$ROOT_DIR/shell/sentia-command-not-found.sh"
FIXTURES_DIR="$ROOT_DIR/tests/shell/fixtures"

export SENTIA_EXECUTABLE_SEARCH_PATH="$FIXTURES_DIR/mock-bin"
export SENTIA_COMMAND_INDEX="$FIXTURES_DIR/command-index.tsv"

assert_contains() {
  local haystack=${1:?}
  local needle=${2:?}
  if ! grep -Fq "$needle" <<<"$haystack"; then
    echo "assertion failed: expected [$needle] in output:" >&2
    echo "$haystack" >&2
    exit 1
  fi
}

output=$($CLASSIFIER "sl")
assert_contains "$output" "CLASS=LIKELY_TYPO"
assert_contains "$output" "SUGGESTION=ls"

output=$($CLASSIFIER "htop")
assert_contains "$output" "CLASS=MISSING_PACKAGE"
assert_contains "$output" "PACKAGES=htop"

output=$($CLASSIFIER "how do i list files")
assert_contains "$output" "CLASS=NATURAL_LANGUAGE"
assert_contains "$output" "AI_HINT=ai -- 'how do i list files'"

output=$($CLASSIFIER "zznosuchcmd")
assert_contains "$output" "CLASS=UNKNOWN"

echo "shell classifier tests passed"
