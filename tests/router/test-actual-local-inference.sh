#!/bin/sh
set -eu

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
broker=/run/sentia-local/broker.sock
backend=/run/sentia-inference/llama.sock

if [ ! -S "$broker" ] || [ ! -S "$backend" ]; then
    echo "SKIP: actual Sentia inference runtime is not available" >&2
    exit 77
fi

if [ ! -x "$repo/target/debug/sentia-router" ] || [ ! -x "$repo/target/debug/ai" ]; then
    lock=/home/ubuntu/sentia-linux/artifacts/.locks/heavy.lock
    mkdir -p "$(dirname "$lock")"
    export CARGO_BUILD_JOBS=2
    exec 9>"$lock"
    flock 9
    cargo build --quiet --manifest-path "$repo/Cargo.toml" \
        -p sentia-router -p sentia-client
fi

runtime="$repo/tests/router/.runtime-$$"
config="$runtime/config"
mkdir -m 700 "$runtime" "$config"
router_pid=
cleanup() {
    if [ -n "$router_pid" ]; then
        kill "$router_pid" 2>/dev/null || true
        wait "$router_pid" 2>/dev/null || true
    fi
    rm -f "$runtime/sentia/router.sock" "$config/router.json"
    rmdir "$runtime/sentia" "$config" "$runtime" 2>/dev/null || true
}
trap cleanup EXIT HUP INT TERM

export XDG_RUNTIME_DIR="$runtime"
export XDG_CONFIG_HOME="$config"
export SENTIA_BROKER_SOCKET="$broker"
"$repo/target/debug/sentia-router" &
router_pid=$!

i=0
while [ ! -S "$runtime/sentia/router.sock" ]; do
    i=$((i + 1))
    if [ "$i" -gt 100 ]; then
        echo "router socket did not become ready" >&2
        exit 1
    fi
    sleep 0.1
done

status=$("$repo/target/debug/ai" status --json)
printf '%s\n' "$status" | grep -q '"frame_type":"result"'
printf '%s\n' "$status" | grep -q 'application/vnd.sentia.status.v1+json'

answer=$("$repo/target/debug/ai" --json "Reply with exactly: sentia-local-ok")
printf '%s\n' "$answer" | grep -q '"event_type":"output_delta"'
printf '%s\n' "$answer" | grep -q '"frame_type":"result".*"status":"completed"'
echo "PASS: actual local llama inference completed through broker and router"
