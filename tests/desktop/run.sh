#!/bin/sh
set -eu

"$(dirname -- "$0")/test-desktop-packaging.sh"
"$(dirname -- "$0")/test-calamares-config.sh"

echo "all desktop tests passed"
