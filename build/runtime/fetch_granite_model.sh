#!/usr/bin/env bash
set -euo pipefail

source "$(dirname "$0")/common.sh"

require_tools curl python3 sha256sum stat

MODEL_REPO="ibm-granite/granite-4.2-3b-GGUF"
MODEL_REVISION="c40945d71cd90f249a56985e8155551a9188dc30"
MODEL_FILE="granite-4.2-3b-Q4_K_M.gguf"
MODEL_SHA256="e0406663965846ae22a403456eb826ccce5f450840491f71952f18a7cb78e7d5"
MODEL_BYTES="2244011552"

MODEL_DIR="${SENTIA_DOWNLOAD_ROOT}/granite-model"
LOG_DIR="${SENTIA_BUILD_ROOT}/logs"
mkdir -p "${MODEL_DIR}" "${LOG_DIR}"

BASE_URL="https://huggingface.co/${MODEL_REPO}/resolve/${MODEL_REVISION}"
TREE_URL="https://huggingface.co/api/models/${MODEL_REPO}/tree/${MODEL_REVISION}?recursive=1"

METADATA_ONLY=0
if [[ "${1:-}" == "--metadata-only" ]]; then
  METADATA_ONLY=1
fi

log "downloading Granite metadata"
curl -fL --retry 4 --retry-delay 3 --output "${MODEL_DIR}/README.md" "${BASE_URL}/README.md"
curl -fL --retry 4 --retry-delay 3 --output "${MODEL_DIR}/model.sig" "${BASE_URL}/model.sig"
curl -fL --retry 4 --retry-delay 3 --output "${MODEL_DIR}/tree.json" "${TREE_URL}"

if [[ "${METADATA_ONLY}" -eq 0 ]]; then
  log "downloading Granite model ${MODEL_FILE}"
  curl -fL --retry 4 --retry-delay 5 -C - --output "${MODEL_DIR}/${MODEL_FILE}" "${BASE_URL}/${MODEL_FILE}"
fi

VERIFY_REPORT="${MODEL_DIR}/verification-report.json"
VERIFY_ARGS=(
  --model "${MODEL_DIR}/${MODEL_FILE}"
  --sig "${MODEL_DIR}/model.sig"
  --readme "${MODEL_DIR}/README.md"
  --tree-json "${MODEL_DIR}/tree.json"
  --report "${VERIFY_REPORT}"
)
if [[ "${METADATA_ONLY}" -eq 1 ]]; then
  VERIFY_ARGS+=(--allow-missing-model)
fi

python3 "${SENTIA_RUNTIME_REPO_ROOT}/build/runtime/verify_granite_model.py" "${VERIFY_ARGS[@]}"

if [[ "${METADATA_ONLY}" -eq 0 ]]; then
  actual_size="$(stat -c%s "${MODEL_DIR}/${MODEL_FILE}")"
  actual_sha="$(sha256sum "${MODEL_DIR}/${MODEL_FILE}" | awk '{print $1}')"
  if [[ "${actual_size}" != "${MODEL_BYTES}" ]]; then
    echo "size mismatch: expected ${MODEL_BYTES}, got ${actual_size}" >&2
    exit 1
  fi
  if [[ "${actual_sha}" != "${MODEL_SHA256}" ]]; then
    echo "sha256 mismatch: expected ${MODEL_SHA256}, got ${actual_sha}" >&2
    exit 1
  fi
fi

sha256sum "${MODEL_DIR}/README.md" "${MODEL_DIR}/model.sig" > "${MODEL_DIR}/metadata.sha256"
if [[ -f "${MODEL_DIR}/${MODEL_FILE}" ]]; then
  sha256sum "${MODEL_DIR}/${MODEL_FILE}" >> "${MODEL_DIR}/metadata.sha256"
fi
cp "${VERIFY_REPORT}" "${LOG_DIR}/granite-verification-report.json"

log "Granite artifacts ready in ${MODEL_DIR}"
