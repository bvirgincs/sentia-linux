#!/bin/sh
set -eu

"$(dirname -- "$0")/test-desktop-packaging.sh"
"$(dirname -- "$0")/test-calamares-config.sh"
"$(dirname -- "$0")/test-run-calamares-root.sh"
"$(dirname -- "$0")/test-staging-contract.sh"

echo "all desktop tests passed"
