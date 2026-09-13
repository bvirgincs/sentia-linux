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
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

init_dirs
publish_log="${LOG_DIR}/publish-$(timestamp_utc).log"
exec > >(tee -a "${publish_log}") 2>&1

log "publish log: ${publish_log}"

require_command gh
require_command sha256sum

iso_path="$(latest_iso_path || true)"
[[ -n "${iso_path}" ]] || die "no ISO artifact found in ${ISO_STAGE_DIR}; run make iso first"

tag="${SENTIA_RELEASE_TAG:-iso-$(date -u +%Y%m%d-%H%M%S)}"
# A pre-release by default: nothing is a Sentia release until it has passed the
# install acceptance tests, and publishing an artifact is not that claim.
prerelease_flag="--prerelease"
[[ "${SENTIA_RELEASE_FINAL:-0}" == "1" ]] && prerelease_flag=""

stage="${SENTIA_BUILDER_ARTIFACTS_DIR}/publish/${tag}"
rm -rf "${stage}"
mkdir -p "${stage}"

log "splitting $(basename "${iso_path}") for release ${tag}"
# 1900 MiB keeps each part clear of GitHub's 2 GiB asset ceiling.
"${REPO_ROOT}/build/release/release_artifacts.py" split "${iso_path}" \
  --output-dir "${stage}" \
  --chunk-size $((1900 * 1024 * 1024))

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

if gh release view "${tag}" >/dev/null 2>&1; then
  log "release ${tag} exists; replacing its assets"
  gh release upload "${tag}" "${stage}"/* --clobber
else
  log "creating release ${tag}"
  # shellcheck disable=SC2086  # prerelease_flag is intentionally unquoted
  gh release create "${tag}" "${stage}"/* \
    --title "Sentia Linux ISO ${tag}" \
    --notes-file "${notes}" \
    ${prerelease_flag}
fi

{
  echo "generated_at=$(timestamp_utc)"
  echo "tag=${tag}"
  echo "iso_path=${iso_path}"
  echo "iso_bytes=${iso_bytes}"
  echo "iso_sha256=${original_sum}"
} | write_atomic "${MANIFEST_DIR}/publish.txt"

log "published ${tag}: $(gh release view "${tag}" --json url --jq .url)"
