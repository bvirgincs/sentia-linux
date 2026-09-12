#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

"$SCRIPT_DIR/build-base-files.sh"
"$SCRIPT_DIR/build-sentia-core.sh"
"$SCRIPT_DIR/build-keyring.sh"
"$SCRIPT_DIR/build-desktop-stack.sh"
# The AI stack is the product, not an optional extra: an image without it is
# not Sentia. Built last because it is by far the most expensive step.
"$SCRIPT_DIR/build-ai-stack.sh"
