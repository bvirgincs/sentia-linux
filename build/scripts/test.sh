#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

"${repo_root}/build/scripts/test-rust.sh"
"${repo_root}/build/scripts/test-failure.sh"
"${repo_root}/build/scripts/test-iso.sh"
"${repo_root}/build/scripts/test-install.sh"
