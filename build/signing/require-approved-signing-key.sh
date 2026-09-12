#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "usage: require-approved-signing-key.sh <fingerprint> [development|production]" >&2
  exit 1
fi

fingerprint="$1"
mode="${2:-development}"
signing_home="${SENTIA_SIGNING_HOME:-$HOME/.local/share/sentia-dev-signing}"
dev_fpr_file="$signing_home/dev-key.fingerprint"
approval_file="${SENTIA_PRODUCTION_KEY_APPROVAL_FILE:-$signing_home/approvals/production-key-approved.txt}"

if [[ "$mode" == "development" ]]; then
  exit 0
fi

if [[ "$mode" != "production" ]]; then
  echo "unsupported signing gate mode: $mode" >&2
  exit 1
fi

if [[ -f "$dev_fpr_file" ]] && grep -Fqx "$fingerprint" "$dev_fpr_file"; then
  echo "refusing production publish with development signing key" >&2
  exit 1
fi

if [[ -z "${SENTIA_OWNER_APPROVED_PRODUCTION_KEY_FPR:-}" ]]; then
  echo "missing SENTIA_OWNER_APPROVED_PRODUCTION_KEY_FPR for production publish" >&2
  exit 1
fi

if [[ "$fingerprint" != "$SENTIA_OWNER_APPROVED_PRODUCTION_KEY_FPR" ]]; then
  echo "signing key fingerprint does not match owner-approved production fingerprint" >&2
  exit 1
fi

if [[ ! -f "$approval_file" ]]; then
  echo "owner production-key approval file not found: $approval_file" >&2
  exit 1
fi

if ! grep -Fqx "$fingerprint" "$approval_file"; then
  echo "production approval file does not authorize fingerprint: $fingerprint" >&2
  exit 1
fi
