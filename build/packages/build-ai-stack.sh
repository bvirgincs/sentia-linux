#!/usr/bin/env bash
# Builds the Sentia AI stack: the Rust workspace (sentia-router,
# sentia-system-tools, sentia-terminal, sentia-health, sentia-providers,
# sentia-firstboot), the llama.cpp local runtime and the pinned Granite model.
#
# Without these packages the ISO has no AI, which is the entire product, so the
# default is to build all of them. The model payload is a 2.2 GB download; set
# SENTIA_SKIP_MODEL_PACKAGE=1 to build everything except sentia-granite-model
# when iterating on something unrelated.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=build/packages/lib.sh
source "$SCRIPT_DIR/lib.sh"

BUILD_MODE="${SENTIA_PACKAGE_BUILD_MODE:-binary}"
ARTIFACT_ROOT="${SENTIA_REPO_ROOT}/artifacts/packages"

build_rust_stack() {
  local work_root="$SENTIA_REPO_ROOT/.build/packages/ai"
  local output_dir="$ARTIFACT_ROOT/ai"

  "$SENTIA_REPO_ROOT/build/packages/stage-input-manifest.sh" \
    "$SENTIA_REPO_ROOT/build/packages/manifests/ai.manifest" "$work_root"

  local source_dir="$work_root/packaging/ai"
  [[ -d "$source_dir" ]] || {
    echo "staged source package missing: $source_dir" >&2
    exit 1
  }

  acquire_heavy_lock
  run_dpkg_buildpackage "$source_dir" "$BUILD_MODE"
  copy_deb_outputs "$work_root/packaging" "$output_dir"
}

build_llama_runtime() {
  local output_dir="$ARTIFACT_ROOT/llama-cpp"
  # build_llama_deb.sh already stages the pinned upstream tree, overlays the
  # Sentia debian/ directory and takes the heavy lock itself.
  "$SENTIA_REPO_ROOT/build/runtime/build_llama_deb.sh"
  mkdir -p "$output_dir"
  find "$SENTIA_REPO_ROOT/.build/runtime/llama-cpp/packages" -maxdepth 1 \
    -type f -name '*.deb' -exec cp -f {} "$output_dir/" \;
}

build_granite_model() {
  local output_dir="$ARTIFACT_ROOT/granite-model"
  "$SENTIA_REPO_ROOT/build/runtime/build_granite_model_deb.sh"
  mkdir -p "$output_dir"
  find "$SENTIA_REPO_ROOT/.build/runtime/granite-model/packages" -maxdepth 1 \
    -type f -name '*.deb' -exec cp -f {} "$output_dir/" \;
}

build_rust_stack
build_llama_runtime

if [[ "${SENTIA_SKIP_MODEL_PACKAGE:-0}" == "1" ]]; then
  echo "SENTIA_SKIP_MODEL_PACKAGE=1: skipping sentia-granite-model" >&2
else
  build_granite_model
fi
