#!/bin/sh
set -eu

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
lock=/home/ubuntu/sentia-linux/artifacts/.locks/heavy.lock
mkdir -p "$(dirname "$lock")"

export CARGO_BUILD_JOBS=2
exec 9>"$lock"
flock 9

for crate in sentia-router sentia-local-broker sentia-client; do
    cargo test --quiet --manifest-path "$repo/crates/$crate/Cargo.toml"
done
