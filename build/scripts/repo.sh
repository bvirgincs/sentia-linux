#!/usr/bin/env bash
# Publishes the signed Sentia overlay archives.
#
# sentia-offline-repository embeds the compact archive, so it cannot be built
# during `make packages` alongside everything else. The archives are therefore
# built in two passes:
#
#   1. build the complete and compact archives from the overlay packages
#   2. build sentia-offline-repository from the compact archive
#   3. rebuild both archives so the complete archive carries the offline
#      package while the compact archive still excludes it
#
# Archive signing needs the secret key, so it runs on the host. The
# offline-repository package build runs inside the Debian 13 builder like every
# other package.
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

init_dirs
repo_log="${LOG_DIR}/repo-$(timestamp_utc).log"
exec > >(tee -a "${repo_log}") 2>&1

log "repo log: ${repo_log}"

require_command reprepro
require_command gpg
require_command rsync

signing_key_id="${SENTIA_REPO_SIGNING_KEY_ID:-}"
[[ -n "${signing_key_id}" ]] || die "missing SENTIA_REPO_SIGNING_KEY_ID (signing agent must supply key id in host keyring)"

signing_public_key="${ARCHIVE_PUBLIC_KEYRING}"
require_file "${signing_public_key}"

[[ -x "${REPOSITORY_BUILD_ENTRYPOINT}" ]] ||
  die "repository build entrypoint not found or not executable: ${REPOSITORY_BUILD_ENTRYPOINT}"

require_dir_nonempty "${PACKAGE_INPUT_DIR}"

export GNUPGHOME="${SIGNING_HOME_DIR}/gnupg"

build_archives() {
  local label="$1"
  run_heavy "${label}" env \
    SENTIA_SIGNING_KEY_FPR="${signing_key_id}" \
    SENTIA_ARCHIVE_SUITE="${ARCHIVE_SUITE}" \
    SENTIA_PACKAGE_INPUT_DIR="${PACKAGE_INPUT_DIR}" \
    SENTIA_REPOSITORY_OUTPUT_DIR="${REPOSITORY_OUTPUT_DIR}" \
    GNUPGHOME="${GNUPGHOME}" \
    "${REPOSITORY_BUILD_ENTRYPOINT}"
}

build_archives "build signed archives (pass 1)"

offline_entrypoint="${REPO_ROOT}/build/packages/build-offline-repository.sh"
[[ -x "${offline_entrypoint}" ]] || die "offline repository build script missing: ${offline_entrypoint}"

if [[ "${SENTIA_PACKAGE_BUILD_IN_BUILDER:-1}" == "1" ]]; then
  SENTIA_BUILDER_WORKSPACE="${REPO_ROOT}" \
  SENTIA_BUILDER_WORKSPACE_MODE=rw \
  run_in_builder "build sentia-offline-repository" "
set -euo pipefail
export SENTIA_HEAVY_LOCK=/artifacts/.locks/heavy.lock
exec '/workspace/build/packages/build-offline-repository.sh'
"
else
  "${offline_entrypoint}"
fi

build_archives "build signed archives (pass 2)"

complete_dir="${REPOSITORY_OUTPUT_DIR}/complete"
[[ -d "${complete_dir}/dists/${ARCHIVE_SUITE}" ]] ||
  die "complete archive metadata missing: ${complete_dir}/dists/${ARCHIVE_SUITE}"

publish_dir="${REPO_STAGE_DIR}/publish"
mkdir -p "${publish_dir}"
rsync -a --delete "${complete_dir}/" "${publish_dir}/"

mkdir -p "${REPO_STAGE_DIR}/public"
cp -f "${signing_public_key}" "${REPO_STAGE_DIR}/public/sentia-archive-keyring.gpg"
chmod 0644 "${REPO_STAGE_DIR}/public/sentia-archive-keyring.gpg"

(
  find "${publish_dir}/dists/${ARCHIVE_SUITE}" -type f | sort
) | while IFS= read -r metadata_file; do
  sha256sum "${metadata_file}"
done | write_atomic "${MANIFEST_DIR}/repo-metadata.sha256"

{
  echo "generated_at=$(timestamp_utc)"
  echo "archive_suite=${ARCHIVE_SUITE}"
  echo "publish_dir=${publish_dir}"
  echo "signing_key_id=${signing_key_id}"
} | write_atomic "${MANIFEST_DIR}/repo-build.txt"

log "signed repo published at: ${publish_dir}"
log "repo metadata checksums: ${MANIFEST_DIR}/repo-metadata.sha256"
