#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

init_dirs
test_failure_log="${LOG_DIR}/test-failure-$(timestamp_utc).log"
exec > >(tee -a "${test_failure_log}") 2>&1

log "test-failure log: ${test_failure_log}"

sandbox="${SENTIA_BUILDER_ARTIFACTS_DIR}/test-failure"
rm -rf "${sandbox}"
mkdir -p "${sandbox}/empty"

run_expect_fail() {
  local label="$1"
  local expected_fragment="$2"
  shift 2

  local output_file="${sandbox}/${label}.log"
  set +e
  "$@" >"${output_file}" 2>&1
  local rc=$?
  set -e

  if [[ "${rc}" -eq 0 ]]; then
    cat "${output_file}" >&2
    die "${label} unexpectedly succeeded"
  fi

  local matched=0
  for _ in {1..20}; do
    if grep -Fq "${expected_fragment}" "${output_file}"; then
      matched=1
      break
    fi
    sleep 0.1
  done

  if [[ "${matched}" -ne 1 ]]; then
    cat "${output_file}" >&2
    die "${label} failed without expected error fragment: ${expected_fragment}"
  fi

  log "${label}: expected failure observed"
}

stub_entrypoint="${sandbox}/no-op-package-build.sh"
cat > "${stub_entrypoint}" <<'STUB'
#!/usr/bin/env bash
# Produces no packages so packages.sh fail-closed validation can be exercised
# without running a real Debian package build.
exit 0
STUB
chmod 0755 "${stub_entrypoint}"

run_expect_fail \
  "packages-missing-inputs" \
  "required directory is empty" \
  env SENTIA_PACKAGE_INPUT_DIR="${sandbox}/empty" \
      SENTIA_PACKAGE_BUILD_ENTRYPOINT="${stub_entrypoint}" \
      SENTIA_PACKAGE_BUILD_IN_BUILDER=0 \
      "${REPO_ROOT}/build/scripts/packages.sh"

run_expect_fail \
  "repo-missing-signing-key" \
  "missing SENTIA_REPO_SIGNING_KEY_ID" \
  env SENTIA_SIGNING_INPUT_DIR="${sandbox}/empty" "${REPO_ROOT}/build/scripts/repo.sh"

run_expect_fail \
  "iso-missing-package-lists" \
  "required directory is empty" \
  env SENTIA_LIVEBUILD_PACKAGE_LIST_DIR="${sandbox}/empty" "${REPO_ROOT}/build/scripts/iso.sh"

{
  echo "generated_at=$(timestamp_utc)"
  echo "sandbox=${sandbox}"
} | write_atomic "${MANIFEST_DIR}/test-failure.txt"

log "fail-closed checks complete"
