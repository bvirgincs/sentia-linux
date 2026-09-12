#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
manifest="$repo_root/crates/sentia-providers/Cargo.toml"
target_dir="${CARGO_TARGET_DIR:-$repo_root/.build/providers-target}"
lock="${SENTIA_HEAVY_LOCK:-/home/ubuntu/sentia-linux/artifacts/.locks/heavy.lock}"

if [ ! -e "$lock" ]; then
    echo "shared heavy-job lock is unavailable: $lock" >&2
    exit 1
fi

unset OPENAI_API_KEY ANTHROPIC_API_KEY GEMINI_API_KEY GOOGLE_API_KEY
unset XAI_API_KEY DEEPSEEK_API_KEY HTTP_PROXY HTTPS_PROXY ALL_PROXY NO_PROXY
unset http_proxy https_proxy all_proxy no_proxy

flock "$lock" env CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR="$target_dir" \
    cargo test --locked --manifest-path "$manifest"

worker="$target_dir/debug/sentia-provider-worker"
capabilities=$("$worker" --provider openai --capabilities)
printf '%s\n' "$capabilities" | grep -q '"live_tested":false'
printf '%s\n' "$capabilities" | grep -q '"enabled_by_default":false'
printf '%s\n' "$capabilities" | grep -q '"separately_billed_api_usage"'
printf '%s\n' "$capabilities" | grep -q '"not_an_api_billing_entitlement"'

events=$(
    printf '%s\n' \
        '{"type":"infer","protocol_version":1,"request":{"request_id":"offline-check","provider":"openai","model":"../blocked","messages":[{"role":"user","content":"fixture"}],"max_output_tokens":8,"stream":false}}' |
        "$worker" --provider openai 3<<'CREDENTIAL'
fixture-secret-never-emit
CREDENTIAL
)
printf '%s\n' "$events" | grep -q '"type":"ready"'
printf '%s\n' "$events" | grep -q '"code":"invalid_model"'
if printf '%s\n' "$events" | grep -q 'fixture-secret-never-emit'; then
    echo "fixture credential leaked into worker output" >&2
    exit 1
fi

echo "provider offline tests passed; no provider request was made"
