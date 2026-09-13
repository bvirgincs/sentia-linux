#!/usr/bin/env bash
# Publish the built ISO to a GitHub release.
#
# The ISO is built on disposable hardware. Until it is published it exists in
# exactly one place, and when that host's lease expires the build is lost. This
# runs as part of the build, not as a manual afterthought.
#
# GitHub caps a release asset at 2 GiB and the ISO is larger, so it is split
# into deterministic parts with a manifest by build/release/release_artifacts.py
# and reassembled by the same tool.
#
# Staging and uploading are separate so the two do not have to happen on the
# same machine. A disposable build runner holds the ISO but must never hold a
# GitHub credential, so it runs "stage" and the authenticated workstation runs
# "upload" against the transferred directory.
#
#   publish-iso.sh            stage and upload from this host
#   publish-iso.sh stage      split and verify only; no credential required
#   publish-iso.sh upload DIR upload an already staged directory
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

mode="${1:-all}"
case "${mode}" in
  all | stage) ;;
  upload)
    [[ $# -ge 2 ]] || { echo "usage: $0 upload <stage-dir>" >&2; exit 2; }
    upload_stage="$2"
    ;;
  *)
    echo "usage: $0 [all|stage|upload <stage-dir>]" >&2
    exit 2
    ;;
esac

init_dirs
publish_log="${LOG_DIR}/publish-$(timestamp_utc).log"
exec > >(tee -a "${publish_log}") 2>&1

log "publish log: ${publish_log}"

require_command sha256sum
[[ "${mode}" == "stage" ]] || require_command gh

# A pre-release by default: nothing is a Sentia release until it has passed the
# install acceptance tests, and publishing an artifact is not that claim.
prerelease_flag="--prerelease"
[[ "${SENTIA_RELEASE_FINAL:-0}" == "1" ]] && prerelease_flag=""

publish_release() {
  local tag="$1" dir="$2" notes="$3" attempt
  # GitHub's release API returns 5xx often enough that a single failure is not
  # evidence of a real problem, and it sometimes applies the change anyway, so
  # re-check before retrying rather than assuming the attempt did nothing.
  for attempt in 1 2 3 4 5; do
    if gh release view "${tag}" >/dev/null 2>&1; then
      log "release ${tag} exists; uploading its assets"
      gh release upload "${tag}" "${dir}"/* --clobber && return 0
    else
      log "creating release ${tag} (attempt ${attempt})"
      # shellcheck disable=SC2086  # prerelease_flag is intentionally unquoted
      gh release create "${tag}" "${dir}"/* \
        --title "Sentia Linux ISO ${tag}" \
        --notes-file "${notes}" \
        ${prerelease_flag} && return 0
    fi
    log "publish attempt ${attempt} failed; retrying"
    sleep $((attempt * 15))
  done
  die "could not publish release ${tag} after 5 attempts"
}

# Uploading a directory staged elsewhere needs none of the split machinery; the
# tag and the notes are already recorded in the directory itself.
if [[ "${mode}" == "upload" ]]; then
  [[ -d "${upload_stage}" ]] || die "no such stage directory: ${upload_stage}"
  upload_stage="$(cd "${upload_stage}" && pwd)"
  tag="$(basename "${upload_stage}")"
  notes="${upload_stage}/NOTES.md"
  [[ -f "${notes}" ]] || die "stage directory has no NOTES.md: ${upload_stage}"
  [[ -f "${upload_stage}/SHA256SUMS" ]] ||
    die "stage directory has no SHA256SUMS: ${upload_stage}"
  manifest="$(find "${upload_stage}" -maxdepth 1 -name '*.json' -type f | head -n 1)"
  [[ -n "${manifest}" ]] || die "stage directory has no manifest: ${upload_stage}"

  # The parts crossed a network to get here. Prove they still reconstruct the
  # ISO before they are published, not after someone downloads them.
  log "verifying the staged parts before upload"
  "${REPO_ROOT}/build/release/release_artifacts.py" verify "${manifest}" \
    --parts-dir "${upload_stage}"

  publish_release "${tag}" "${upload_stage}" "${notes}"
  log "published ${tag}: $(gh release view "${tag}" --json url --jq .url)"
  exit 0
fi

iso_path="$(latest_iso_path || true)"
[[ -n "${iso_path}" ]] || die "no ISO artifact found in ${ISO_STAGE_DIR}; run make iso first"

tag="${SENTIA_RELEASE_TAG:-iso-$(date -u +%Y%m%d-%H%M%S)}"

stage="${SENTIA_BUILDER_ARTIFACTS_DIR}/publish/${tag}"
rm -rf "${stage}"
mkdir -p "${stage}"

log "splitting $(basename "${iso_path}") for release ${tag}"
# release_artifacts.py caps a part at 1 GiB, comfortably under GitHub's 2 GiB
# asset ceiling, and defaults to that maximum.
"${REPO_ROOT}/build/release/release_artifacts.py" split "${iso_path}" \
  --output-dir "${stage}"

manifest="$(find "${stage}" -maxdepth 1 -name '*.json' -type f | head -n 1)"
[[ -n "${manifest}" ]] || die "the split produced no manifest"

# Reassembling from the published parts is the only thing a user actually
# needs to work, so prove it here rather than after someone downloads 3.6 GB.
log "verifying the split reconstructs the ISO byte for byte"
"${REPO_ROOT}/build/release/release_artifacts.py" verify "${manifest}" --parts-dir "${stage}"
rebuilt="${stage}/reconstructed.iso"
"${REPO_ROOT}/build/release/release_artifacts.py" join "${manifest}" \
  --parts-dir "${stage}" --output "${rebuilt}"
original_sum="$(sha256sum "${iso_path}" | awk '{print $1}')"
rebuilt_sum="$(sha256sum "${rebuilt}" | awk '{print $1}')"
[[ "${original_sum}" == "${rebuilt_sum}" ]] ||
  die "reconstruction mismatch: ${original_sum} != ${rebuilt_sum}"
rm -f "${rebuilt}"
log "reconstruction verified: ${original_sum}"

printf '%s  %s\n' "${original_sum}" "$(basename "${iso_path}")" \
  > "${stage}/SHA256SUMS"

iso_bytes="$(stat -c %s "${iso_path}")"
notes="${stage}/NOTES.md"
cat > "${notes}" <<EOF
# Sentia Linux ISO — ${tag}

\`$(basename "${iso_path}")\`, ${iso_bytes} bytes, sha256 \`${original_sum}\`.

GitHub caps a release asset at 2 GiB, so the ISO is published as split parts
with a manifest. Reassemble it with the tool from this repository:

\`\`\`sh
gh release download ${tag} --dir sentia-iso
./build/release/release_artifacts.py join \\
  sentia-iso/$(basename "${manifest}") \\
  --parts-dir sentia-iso \\
  --output $(basename "${iso_path}")
sha256sum --check sentia-iso/SHA256SUMS
\`\`\`

Verify the checksum before writing the image to anything.

Build metadata: $(git -C "${REPO_ROOT}" rev-parse HEAD 2>/dev/null || echo unknown)
EOF

if [[ "${mode}" == "stage" ]]; then
  log "staged ${tag} in ${stage}; upload it with: $0 upload <dir>"
else
  publish_release "${tag}" "${stage}" "${notes}"
fi

{
  echo "generated_at=$(timestamp_utc)"
  echo "tag=${tag}"
  echo "iso_path=${iso_path}"
  echo "iso_bytes=${iso_bytes}"
  echo "iso_sha256=${original_sum}"
} | write_atomic "${MANIFEST_DIR}/publish.txt"

[[ "${mode}" == "stage" ]] ||
  log "published ${tag}: $(gh release view "${tag}" --json url --jq .url)"
