#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
MANIFEST_PATH="$ROOT_DIR/crates/sentia-ui/Cargo.toml"
LOCK_FILE="/home/ubuntu/sentia-linux/artifacts/.locks/heavy.lock"

if ! command -v cargo >/dev/null 2>&1; then
  echo "SKIP: cargo not available"
  exit 0
fi

if ! command -v xvfb-run >/dev/null 2>&1; then
  echo "SKIP: xvfb-run not available"
  exit 0
fi

exec 9>"$LOCK_FILE"
flock -x 9

cargo build --manifest-path "$MANIFEST_PATH" --bins -j2

xvfb-run -a "$ROOT_DIR/crates/sentia-ui/target/debug/sentia-terminal" --conventional --quit-after-ms 1200
xvfb-run -a "$ROOT_DIR/crates/sentia-ui/target/debug/sentia-monitor" --quit-after-ms 1200
xvfb-run -a "$ROOT_DIR/crates/sentia-ui/target/debug/sentia-firstboot" --quit-after-ms 1200

echo "UI launch smoke tests passed"
