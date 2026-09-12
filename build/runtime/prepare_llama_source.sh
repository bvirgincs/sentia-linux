#!/usr/bin/env bash
set -euo pipefail

source "$(dirname "$0")/common.sh"

require_tools curl tar sha256sum python3

LLAMA_COMMIT="5266f24da75dc449bd56cbed7addb9c8e4a6a73e"
LLAMA_VERSION="0.4.0+git5266f24"
LLAMA_TARBALL_URL="https://github.com/ggml-org/llama.cpp/archive/${LLAMA_COMMIT}.tar.gz"

DOWNLOAD_DIR="${SENTIA_DOWNLOAD_ROOT}/llama-cpp"
BUILD_DIR="${SENTIA_BUILD_ROOT}/llama-cpp"
SRC_DIR="${BUILD_DIR}/src"
WORK_DIR="${SRC_DIR}/llama.cpp-${LLAMA_VERSION}"
LOG_DIR="${SENTIA_BUILD_ROOT}/logs"

mkdir -p "${DOWNLOAD_DIR}" "${SRC_DIR}" "${LOG_DIR}"

TARBALL="${DOWNLOAD_DIR}/llama.cpp-${LLAMA_COMMIT}.tar.gz"
log "downloading llama.cpp ${LLAMA_COMMIT}"
curl -fL --retry 4 --retry-delay 3 --output "${TARBALL}" "${LLAMA_TARBALL_URL}"

rm -rf "${WORK_DIR}"
mkdir -p "${WORK_DIR}"

tar -xzf "${TARBALL}" --strip-components=1 -C "${WORK_DIR}"

# Overlay Sentia packaging/config/runtime probe files into the staged source tree.
mkdir -p "${WORK_DIR}/config" "${WORK_DIR}/build" "${WORK_DIR}/packaging"
cp -a "${SENTIA_RUNTIME_REPO_ROOT}/config/llama" "${WORK_DIR}/config/"
cp -a "${SENTIA_RUNTIME_REPO_ROOT}/config/systemd" "${WORK_DIR}/config/"
cp -a "${SENTIA_RUNTIME_REPO_ROOT}/build/runtime" "${WORK_DIR}/build/"
cp -a "${SENTIA_RUNTIME_REPO_ROOT}/packaging/llama-cpp/debian" "${WORK_DIR}/"

python3 "${SENTIA_RUNTIME_REPO_ROOT}/build/runtime/inspect_llama_source.py" \
  --output "${LOG_DIR}/llama-source-inspection.json"

sha256sum "${TARBALL}" > "${LOG_DIR}/llama-upstream-tarball.sha256"

echo "${WORK_DIR}"
