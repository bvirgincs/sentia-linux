#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

init_dirs
packages_log="${LOG_DIR}/packages-$(timestamp_utc).log"
exec > >(tee -a "${packages_log}") 2>&1

log "packages log: ${packages_log}"
log "package input directory: ${PACKAGE_INPUT_DIR}"

require_command dpkg-deb
require_dir_nonempty "${PACKAGE_INPUT_DIR}"

shopt -s nullglob
package_candidates=("${PACKAGE_INPUT_DIR}"/*.deb)
if [[ "${#package_candidates[@]}" -eq 0 ]]; then
  die "no .deb files found in package input directory: ${PACKAGE_INPUT_DIR}"
fi

stage_pool="${PACKAGE_STAGE_DIR}/pool"
mkdir -p "${stage_pool}"
find "${stage_pool}" -maxdepth 1 -type f -name '*.deb' -delete

for deb in "${package_candidates[@]}"; do
  cp -f "${deb}" "${stage_pool}/"
done

{
  printf 'package\tversion\tarchitecture\tfilename\tsha256\n'
  for deb in "${stage_pool}"/*.deb; do
    package_name="$(dpkg-deb -f "${deb}" Package)"
    package_version="$(dpkg-deb -f "${deb}" Version)"
    package_arch="$(dpkg-deb -f "${deb}" Architecture)"
    package_sha="$(sha256sum "${deb}" | awk '{print $1}')"
    printf '%s\t%s\t%s\t%s\t%s\n' \
      "${package_name}" \
      "${package_version}" \
      "${package_arch}" \
      "$(basename "${deb}")" \
      "${package_sha}"
  done | sort
} | write_atomic "${MANIFEST_DIR}/staged-packages.tsv"

log "staged package pool: ${stage_pool}"
log "package manifest: ${MANIFEST_DIR}/staged-packages.tsv"
