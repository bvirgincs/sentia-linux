#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

"$SCRIPT_DIR/test-config.sh"
"$SCRIPT_DIR/test-build-manifests.sh"
"$SCRIPT_DIR/test-apt-trust.sh"
