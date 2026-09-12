#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

init_dirs
iso_log="${LOG_DIR}/iso-$(timestamp_utc).log"
exec > >(tee -a "${iso_log}") 2>&1

log "iso log: ${iso_log}"

require_command rsync
require_dir_nonempty "${LIVEBUILD_PACKAGE_LIST_DIR}"
shopt -s nullglob
package_lists=("${LIVEBUILD_PACKAGE_LIST_DIR}"/*.list.chroot "${LIVEBUILD_PACKAGE_LIST_DIR}"/*.list.binary)
if [[ "${#package_lists[@]}" -eq 0 ]]; then
  die "live-build package lists are missing (*.list.chroot or *.list.binary) in ${LIVEBUILD_PACKAGE_LIST_DIR}"
fi

release_suite="${ARCHIVE_SUITE}"
repo_publish_dir="${REPO_STAGE_DIR}/publish"
repo_dists_dir="${repo_publish_dir}/dists/${release_suite}"
[[ -d "${repo_dists_dir}" ]] || die "signed overlay repo metadata not found: ${repo_dists_dir}; run make repo first"

repo_keyring="${REPO_STAGE_DIR}/public/sentia-archive-keyring.gpg"
require_file "${repo_keyring}"

# In-chroot location of the build-time overlay archive; auto/build stages and
# removes it at this same path.
readonly SENTIA_BUILD_ARCHIVE_PATH="/srv/sentia-build-archive"

work_livebuild_dir="${WORK_DIR}/live-build"
run_heavy "reset live-build work directory" sudo bash -lc "
set -euo pipefail
rm -rf '${work_livebuild_dir}'
mkdir -p '${work_livebuild_dir}'
"

rsync -a --delete "${REPO_ROOT}/config/live-build/" "${work_livebuild_dir}/"

# live-build builds its own chroot, which does not inherit the outer builder's
# bind mounts. Stage the signed overlay inside the live-build tree so auto/build
# can copy it into that chroot before packages are installed. The sources entry
# is .list.chroot only: it is build-time trust, and live-build drops it from the
# shipped image. The installed system gets its archive from
# sentia-offline-repository instead.
overlay_stage_dir="${work_livebuild_dir}/sentia-overlay-archive"
rm -rf "${overlay_stage_dir}"
mkdir -p "${overlay_stage_dir}"
rsync -a --delete "${repo_publish_dir}/" "${overlay_stage_dir}/"
cp -f "${repo_keyring}" "${work_livebuild_dir}/sentia-archive-keyring.gpg"

mkdir -p "${work_livebuild_dir}/config/archives"
cat > "${work_livebuild_dir}/config/archives/sentia-overlay.list.chroot.tmp" <<EOF_CHROOT
deb [signed-by=/usr/share/keyrings/sentia-archive-keyring.gpg] file:${SENTIA_BUILD_ARCHIVE_PATH} ${release_suite} main
EOF_CHROOT
mv "${work_livebuild_dir}/config/archives/sentia-overlay.list.chroot.tmp" \
  "${work_livebuild_dir}/config/archives/sentia-overlay.list.chroot"

epoch="$(source_date_epoch)"

iso_build_command="$(cat <<CMD
set -euo pipefail
cd /workspace
[[ -x ./auto/config ]] || { echo 'ERROR: missing /workspace/auto/config' >&2; exit 1; }
if [[ -x ./auto/clean ]]; then
  ./auto/clean || true
else
  lb clean --purge || true
fi
./auto/config
if [[ -x ./auto/build ]]; then
  SOURCE_DATE_EPOCH=${epoch} ./auto/build
else
  SOURCE_DATE_EPOCH=${epoch} lb build --build-with-chroot true
fi
CMD
)"

SENTIA_BUILDER_WORKSPACE="${work_livebuild_dir}" \
SENTIA_BUILDER_WORKSPACE_MODE=rw \
run_in_builder "build live ISO" "${iso_build_command}"

built_iso="$(find "${work_livebuild_dir}" -maxdepth 1 -type f -name '*.iso' | sort | tail -n 1 || true)"
[[ -n "${built_iso}" ]] || die "live-build completed but no ISO was produced under ${work_livebuild_dir}"

iso_output="${ISO_STAGE_DIR}/sentia-trixie-amd64.iso"
cp -f "${built_iso}" "${iso_output}.tmp"
mv "${iso_output}.tmp" "${iso_output}"

sha256sum "${iso_output}" | write_atomic "${MANIFEST_DIR}/iso.sha256"

{
  echo "generated_at=$(timestamp_utc)"
  echo "source_date_epoch=${epoch}"
  echo "iso_output=${iso_output}"
  echo "release_suite=${release_suite}"
} | write_atomic "${MANIFEST_DIR}/iso-build.txt"

(
  for path in "${package_lists[@]}"; do
    sha256sum "${path}"
  done
  find "${REPO_ROOT}/config/live-build/auto" -type f | sort | while IFS= read -r script_path; do
    sha256sum "${script_path}"
  done
  find "${repo_dists_dir}" -type f | sort | while IFS= read -r repo_file; do
    sha256sum "${repo_file}"
  done
) | write_atomic "${MANIFEST_DIR}/iso-inputs.sha256"

log "iso output: ${iso_output}"
log "iso checksum manifest: ${MANIFEST_DIR}/iso.sha256"
