#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

"$SCRIPT_DIR/test-command-not-found.sh"
"$SCRIPT_DIR/test-terminal-hook.sh"
