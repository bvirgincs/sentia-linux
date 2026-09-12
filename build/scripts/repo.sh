#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

init_dirs
repo_log="${LOG_DIR}/repo-$(timestamp_utc).log"
exec > >(tee -a "${repo_log}") 2>&1

log "repo log: ${repo_log}"

require_command reprepro

signing_key_id="${SENTIA_REPO_SIGNING_KEY_ID:-}"
[[ -n "${signing_key_id}" ]] || die "missing SENTIA_REPO_SIGNING_KEY_ID (signing agent must supply key id in host keyring)"

signing_public_key="${SIGNING_INPUT_DIR}/public/sentia-archive-keyring.gpg"
require_file "${signing_public_key}"

stage_pool="${PACKAGE_STAGE_DIR}/pool"
require_dir_nonempty "${stage_pool}"

shopt -s nullglob
staged_packages=("${stage_pool}"/*.deb)
if [[ "${#staged_packages[@]}" -eq 0 ]]; then
  die "no staged .deb packages found: ${stage_pool}"
fi

release_suite="${SENTIA_RELEASE_SUITE:-sentia-trixie-0.1}"
publish_dir="${REPO_STAGE_DIR}/publish"
conf_dir="${publish_dir}/conf"
mkdir -p "${conf_dir}"

cat > "${conf_dir}/distributions.tmp" <<EOF_DIST
Origin: Sentia
Label: Sentia
Suite: ${release_suite}
Codename: ${release_suite}
Architectures: amd64 source
Components: main
Description: Sentia overlay for Debian 13 trixie
SignWith: ${signing_key_id}
Contents: .gz
EOF_DIST
mv "${conf_dir}/distributions.tmp" "${conf_dir}/distributions"

for deb in "${staged_packages[@]}"; do
  run_heavy "include $(basename "${deb}") into ${release_suite}" \
    reprepro --basedir "${publish_dir}" includedeb "${release_suite}" "${deb}"
done

mkdir -p "${REPO_STAGE_DIR}/public"
cp -f "${signing_public_key}" "${REPO_STAGE_DIR}/public/sentia-archive-keyring.gpg"

(
  find "${publish_dir}/dists/${release_suite}" -type f | sort
) | while IFS= read -r metadata_file; do
  sha256sum "${metadata_file}"
done | write_atomic "${MANIFEST_DIR}/repo-metadata.sha256"

log "signed repo published at: ${publish_dir}"
log "repo metadata checksums: ${MANIFEST_DIR}/repo-metadata.sha256"
