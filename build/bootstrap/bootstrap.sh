#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/scripts/common.sh"

init_dirs
bootstrap_log="${LOG_DIR}/bootstrap-$(timestamp_utc).log"
exec > >(tee -a "${bootstrap_log}") 2>&1

log "bootstrap log: ${bootstrap_log}"
log "shared root: ${SENTIA_SHARED_ROOT}"
log "build root: ${SENTIA_BUILD_ROOT}"
log "builder chroot: ${CHROOT_DIR}"

host_manifest="${REPO_ROOT}/build/manifests/host-dependencies.txt"
builder_manifest="${REPO_ROOT}/build/manifests/builder-dependencies.txt"
sources_manifest="${REPO_ROOT}/build/manifests/debian-trixie.sources"

require_file "${host_manifest}"
require_file "${builder_manifest}"
require_file "${sources_manifest}"
require_file "${REPO_ROOT}/build/bootstrap/install-builder-deps.sh"

require_command sudo

mapfile -t host_packages < <(read_manifest_packages "${host_manifest}")
[[ "${#host_packages[@]}" -gt 0 ]] || die "host dependency manifest is empty: ${host_manifest}"

skip_host_apt="${SENTIA_SKIP_HOST_APT:-0}"
if [[ "${skip_host_apt}" == "1" ]]; then
  log "SENTIA_SKIP_HOST_APT=1 set; skipping host apt update/install"
else
  run_heavy "host apt update" sudo env DEBIAN_FRONTEND=noninteractive apt-get update
  run_heavy "host apt install" sudo env DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends "${host_packages[@]}"
fi

require_command debootstrap
require_command systemd-nspawn

if [[ ! -x "${CHROOT_DIR}/bin/bash" ]]; then
  run_heavy "debootstrap Debian 13 trixie" sudo debootstrap \
    --arch=amd64 \
    --variant=minbase \
    --components=main,non-free-firmware \
    --include=ca-certificates,debian-archive-keyring \
    trixie \
    "${CHROOT_DIR}" \
    https://deb.debian.org/debian
else
  log "existing chroot detected; reusing ${CHROOT_DIR}"
fi

sudo bash -lc "
set -euo pipefail
install -d -m 0755 '${CHROOT_DIR}/etc/apt/sources.list.d'
cp '${sources_manifest}' '${CHROOT_DIR}/etc/apt/sources.list.d/debian.sources'
rm -f '${CHROOT_DIR}/etc/apt/sources.list'
"

skip_builder_apt="${SENTIA_SKIP_BUILDER_APT:-0}"
if [[ "${skip_builder_apt}" == "1" ]]; then
  log "SENTIA_SKIP_BUILDER_APT=1 set; skipping builder dependency reinstall"
  if [[ ! -f "${CHROOT_DIR}/.sentia-builder-deps.complete" ]]; then
    sudo bash -lc "echo '$(timestamp_utc)' > '${CHROOT_DIR}/.sentia-builder-deps.complete'"
  fi
elif [[ ! -f "${CHROOT_DIR}/.sentia-builder-deps.complete" ]]; then
  run_in_builder "install builder dependency set" "set -euo pipefail; /workspace/build/bootstrap/install-builder-deps.sh"
  sudo bash -lc "echo '$(timestamp_utc)' > '${CHROOT_DIR}/.sentia-builder-deps.complete'"
else
  log "existing builder dependency marker detected; skipping dependency reinstall"
fi

host_pkg_manifest="${MANIFEST_DIR}/host-packages.tsv"
dpkg-query -W -f='${Package}\t${Version}\n' "${host_packages[@]}" | sort | write_atomic "${host_pkg_manifest}"

{
  echo "rustc_path=$(command -v rustc)"
  rustc --version
  echo "cargo_path=$(command -v cargo)"
  cargo --version
} | write_atomic "${MANIFEST_DIR}/host-rust.txt"

builder_package_capture="$(cat <<'CMD'
set -euo pipefail
tmp="/artifacts/manifests/builder-packages.tsv.tmp.$$"
dpkg-query -W -f='${Package}\t${Version}\n' | sort > "$tmp"
mv "$tmp" /artifacts/manifests/builder-packages.tsv
CMD
)"
run_in_builder_no_lock "capture builder package manifest" "${builder_package_capture}"

builder_rust_capture="$(cat <<'CMD'
set -euo pipefail
tmp="/artifacts/manifests/builder-rust.txt.tmp.$$"
{
  echo "rustc_path=$(command -v rustc)"
  rustc --version
  echo "cargo_path=$(command -v cargo)"
  cargo --version
} > "$tmp"
mv "$tmp" /artifacts/manifests/builder-rust.txt
CMD
)"
run_in_builder_no_lock "capture builder rust toolchain" "${builder_rust_capture}"

builder_index_capture="$(cat <<'CMD'
set -euo pipefail
shopt -s nullglob
declare -A seen=()
files=()
for candidate in /var/lib/apt/lists/*InRelease /var/lib/apt/lists/*Release /var/lib/apt/lists/*Release.gpg; do
  [[ -f "$candidate" ]] || continue
  if [[ -z "${seen[$candidate]:-}" ]]; then
    files+=("$candidate")
    seen["$candidate"]=1
  fi
done
if [[ "${#files[@]}" -eq 0 ]]; then
  echo "ERROR: no apt index files found under /var/lib/apt/lists" >&2
  exit 1
fi
tmp="/artifacts/manifests/builder-apt-indices.sha256.tmp.$$"
sha256sum "${files[@]}" > "$tmp"
mv "$tmp" /artifacts/manifests/builder-apt-indices.sha256
CMD
)"
run_in_builder_no_lock "capture builder apt index checksums" "${builder_index_capture}"

{
  echo "generated_at=$(timestamp_utc)"
  echo "source_date_epoch=$(source_date_epoch)"
  echo "repo_root=${REPO_ROOT}"
  echo "shared_root=${SENTIA_SHARED_ROOT}"
  echo "chroot_dir=${CHROOT_DIR}"
} | write_atomic "${MANIFEST_DIR}/bootstrap-environment.txt"

(
  cd "${REPO_ROOT}"
  {
    find build/bootstrap -type f
    find build/manifests -type f
    find build/scripts -type f
    find config/live-build/auto -type f
    printf '%s\n' Makefile docs/BUILD.md
  } | sort -u
) | while IFS= read -r rel_path; do
  sha256sum "${REPO_ROOT}/${rel_path}"
done | write_atomic "${MANIFEST_DIR}/builder-input-source.sha256"

sudo bash -lc "
set -euo pipefail
echo '$(timestamp_utc)' > '${CHROOT_DIR}/.sentia-bootstrap.complete'
"

# The development archive signing key lives outside the repository and outside
# every build output. Only the exported public keyring is ever made visible to
# the builder; the secret key stays in the host signing home.
require_command gpg
dev_fingerprint="$("${REPO_ROOT}/build/signing/init-dev-key.sh")"
log "development signing fingerprint: ${dev_fingerprint}"
log "development signing home: ${SIGNING_HOME_DIR}"

log "bootstrap complete"
log "host rust path: $(command -v rustc)"
log "host cargo path: $(command -v cargo)"
log "manifests: ${MANIFEST_DIR}"
