#!/usr/bin/env bash
set -euo pipefail

source "$(dirname "$0")/common.sh"

require_tools dpkg-deb python3 install

MODEL_DIR="${SENTIA_DOWNLOAD_ROOT}/granite-model"
MODEL_FILE="${MODEL_DIR}/granite-4.2-3b-Q4_K_M.gguf"
MODEL_SIG="${MODEL_DIR}/model.sig"
MODEL_CARD="${MODEL_DIR}/README.md"
TREE_JSON="${MODEL_DIR}/tree.json"
VERIFY_REPORT="${MODEL_DIR}/verification-report.json"

if [[ ! -f "${MODEL_FILE}" || ! -f "${MODEL_SIG}" || ! -f "${MODEL_CARD}" || ! -f "${TREE_JSON}" ]]; then
  "${SENTIA_RUNTIME_REPO_ROOT}/build/runtime/fetch_granite_model.sh"
fi

python3 "${SENTIA_RUNTIME_REPO_ROOT}/build/runtime/verify_granite_model.py" \
  --model "${MODEL_FILE}" \
  --sig "${MODEL_SIG}" \
  --readme "${MODEL_CARD}" \
  --tree-json "${TREE_JSON}" \
  --report "${VERIFY_REPORT}"

BUILD_DIR="${SENTIA_BUILD_ROOT}/granite-model"
PKG_DIR="${BUILD_DIR}/pkgroot"
PKG_OUT="${BUILD_DIR}/packages"
LOG_DIR="${SENTIA_BUILD_ROOT}/logs"
mkdir -p "${PKG_DIR}" "${PKG_OUT}" "${LOG_DIR}"
rm -rf "${PKG_DIR}"
mkdir -p "${PKG_DIR}/DEBIAN" "${PKG_DIR}/usr/share/sentia/models/granite-4.2-3b" "${PKG_DIR}/usr/share/doc/sentia-granite-model"

cat > "${PKG_DIR}/DEBIAN/control" <<'CONTROL'
Package: sentia-granite-model
Version: 4.2.3b+rev.c40945d7-1
Section: misc
Priority: optional
Architecture: amd64
Maintainer: Sentia Maintainers <maintainers@sentia.local>
Description: IBM Granite 4.2 3B GGUF model for Sentia local runtime
 Exact pinned GGUF payload for offline Sentia inference:
 - repository: ibm-granite/granite-4.2-3b-GGUF
 - revision: c40945d71cd90f249a56985e8155551a9188dc30
 - file: granite-4.2-3b-Q4_K_M.gguf
CONTROL

install -m 0644 "${MODEL_FILE}" "${PKG_DIR}/usr/share/sentia/models/granite-4.2-3b/granite-4.2-3b-Q4_K_M.gguf"
install -m 0644 "${MODEL_SIG}" "${PKG_DIR}/usr/share/sentia/models/granite-4.2-3b/model.sig"
install -m 0644 "${MODEL_CARD}" "${PKG_DIR}/usr/share/sentia/models/granite-4.2-3b/MODEL_CARD.md"
install -m 0644 "${VERIFY_REPORT}" "${PKG_DIR}/usr/share/doc/sentia-granite-model/verification.json"
install -m 0644 "${SENTIA_RUNTIME_REPO_ROOT}/packaging/granite-model/provenance/granite-4.2-3b-q4_k_m.provenance.json" "${PKG_DIR}/usr/share/doc/sentia-granite-model/provenance.json"

OUT_DEB="${PKG_OUT}/sentia-granite-model_4.2.3b+rev.c40945d7-1_amd64.deb"
dpkg-deb --root-owner-group --build "${PKG_DIR}" "${OUT_DEB}" >/dev/null
sha256sum "${OUT_DEB}" > "${LOG_DIR}/granite-model-deb.sha256"

log "granite model package artifact: ${OUT_DEB}"
