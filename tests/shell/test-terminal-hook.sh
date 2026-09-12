#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
HOOK_PATH="$ROOT_DIR/shell/sentia-terminal-hook.bash"
FIXTURES_DIR="$ROOT_DIR/tests/shell/fixtures"

export HOOK_PATH
export SENTIA_EXECUTABLE_SEARCH_PATH="$FIXTURES_DIR/mock-bin"
export SENTIA_COMMAND_INDEX="$FIXTURES_DIR/command-index.tsv"

output=$(bash --noprofile --norc <<'BASH'
set -euo pipefail
set -o history

export _SENTIA_TERMINAL=1
export SENTIA_TERMINAL_ASSIST_MODE=command-not-found
export SENTIA_TERMINAL_CAPTURE=1
export SENTIA_CAPTURE_MAX=2

# shellcheck source=/dev/null
source "$HOOK_PATH"

command_not_found_handle sl 2>&1 || true
history -s "echo one"
__sentia_capture_prompt
history -s "echo two"
__sentia_capture_prompt
history -s "echo three"
__sentia_capture_prompt

echo "CAPTURE_SIZE=${#__sentia_capture_events[@]}"
BASH
)

if ! grep -Fq "likely typo" <<<"$output"; then
  echo "expected typo hint in hook output" >&2
  echo "$output" >&2
  exit 1
fi

if ! grep -Fq "CAPTURE_SIZE=2" <<<"$output"; then
  echo "expected bounded in-memory capture" >&2
  echo "$output" >&2
  exit 1
fi

echo "shell hook tests passed"
