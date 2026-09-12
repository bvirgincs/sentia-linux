#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

init_dirs
packages_log="${LOG_DIR}/packages-$(timestamp_utc).log"
exec > >(tee -a "${packages_log}") 2>&1

log "packages log: ${packages_log}"
log "package build entrypoint: ${PACKAGE_BUILD_ENTRYPOINT}"
log "package output root: ${PACKAGE_INPUT_DIR}"

require_command find
require_command dpkg-deb

[[ -x "${PACKAGE_BUILD_ENTRYPOINT}" ]] || die "package build entrypoint not found or not executable: ${PACKAGE_BUILD_ENTRYPOINT}"

# Debian packages must be produced against Debian 13, not against whatever
# distribution the orchestration host happens to run. Execute the entrypoint
# inside the bootstrapped builder, mirroring iso.sh. The workspace is bound
# read-write because the package scripts stage sources under .build/ and emit
# .deb files under artifacts/packages/, both of which are ignored paths.
if [[ "${SENTIA_PACKAGE_BUILD_IN_BUILDER:-1}" == "1" ]]; then
  entrypoint_in_builder="/workspace${PACKAGE_BUILD_ENTRYPOINT#"${REPO_ROOT}"}"
  [[ "${entrypoint_in_builder}" != "${PACKAGE_BUILD_ENTRYPOINT}" ]] ||
    die "package build entrypoint must live inside the repository to run in the builder: ${PACKAGE_BUILD_ENTRYPOINT}"

  SENTIA_BUILDER_WORKSPACE="${REPO_ROOT}" \
  SENTIA_BUILDER_WORKSPACE_MODE=rw \
  run_in_builder "build Debian packages" "
set -euo pipefail
export SENTIA_HEAVY_LOCK=/artifacts/.locks/heavy.lock
exec '${entrypoint_in_builder}'
"
else
  "${PACKAGE_BUILD_ENTRYPOINT}"
fi

require_dir_nonempty "${PACKAGE_INPUT_DIR}"

shopt -s nullglob
mapfile -t package_candidates < <(find "${PACKAGE_INPUT_DIR}" -type f -name '*.deb' | sort)
if [[ "${#package_candidates[@]}" -eq 0 ]]; then
  die "no .deb files found under package output root: ${PACKAGE_INPUT_DIR}"
fi

stage_pool="${PACKAGE_STAGE_DIR}/pool"
mkdir -p "${stage_pool}"
find "${stage_pool}" -maxdepth 1 -type f -name '*.deb' -delete

for deb in "${package_candidates[@]}"; do
  cp -f "${deb}" "${stage_pool}/"
done

{
  printf 'package\tversion\tarchitecture\tfilename\tsource_path\tsha256\n'
  for deb in "${stage_pool}"/*.deb; do
    package_name="$(dpkg-deb -f "${deb}" Package)"
    package_version="$(dpkg-deb -f "${deb}" Version)"
    package_arch="$(dpkg-deb -f "${deb}" Architecture)"
    package_sha="$(sha256sum "${deb}" | awk '{print $1}')"
    source_path="$(find "${PACKAGE_INPUT_DIR}" -type f -name "$(basename "${deb}")" | sort | head -n 1 || true)"
    printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
      "${package_name}" \
      "${package_version}" \
      "${package_arch}" \
      "$(basename "${deb}")" \
      "${source_path}" \
      "${package_sha}"
  done | sort
} | write_atomic "${MANIFEST_DIR}/staged-packages.tsv"

log "staged package pool: ${stage_pool}"
log "package manifest: ${MANIFEST_DIR}/staged-packages.tsv"
