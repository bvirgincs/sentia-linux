#!/usr/bin/env bash
set -euo pipefail

source "$(dirname "$0")/common.sh"

require_tools dpkg-buildpackage cmake ninja g++ make pkg-config flock

BUILD_DIR="${SENTIA_BUILD_ROOT}/llama-cpp"
PKG_OUT="${BUILD_DIR}/packages"
LOG_DIR="${SENTIA_BUILD_ROOT}/logs"
mkdir -p "${PKG_OUT}" "${LOG_DIR}"

SOURCE_DIR="$(${SENTIA_RUNTIME_REPO_ROOT}/build/runtime/prepare_llama_source.sh | tail -n 1)"

# Two is the correct default on a shared development host, where a local test
# VM competes for memory. A dedicated build runner should raise this.
LLAMA_BUILD_JOBS="${SENTIA_BUILD_JOBS:-2}"

log "building llama.cpp Debian package under heavy lock with ${LLAMA_BUILD_JOBS} jobs"
run_with_heavy_lock bash -lc "cd '${SOURCE_DIR}' && DEB_BUILD_OPTIONS='parallel=${LLAMA_BUILD_JOBS}' dpkg-buildpackage -us -uc -b -j${LLAMA_BUILD_JOBS} -d 2>&1 | tee '${LOG_DIR}/llama-dpkg-build.log'"

find "$(dirname "${SOURCE_DIR}")" -maxdepth 1 -type f -name '*.deb' -print -exec cp -f {} "${PKG_OUT}/" \;
if ! ls -1 "${PKG_OUT}"/*.deb >/dev/null 2>&1; then
  echo "no .deb artifacts produced" >&2
  exit 1
fi

sha256sum "${PKG_OUT}"/*.deb > "${LOG_DIR}/llama-debs.sha256"
log "llama package artifacts: ${PKG_OUT}"
