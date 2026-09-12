#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

"$SCRIPT_DIR/build-base-files.sh"
"$SCRIPT_DIR/build-sentia-core.sh"
"$SCRIPT_DIR/build-keyring.sh"
"$SCRIPT_DIR/build-desktop-stack.sh"
"$SCRIPT_DIR/build-offline-repository.sh"
