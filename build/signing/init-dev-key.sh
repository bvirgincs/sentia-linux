#!/usr/bin/env bash
set -euo pipefail

SIGNING_HOME="${SENTIA_SIGNING_HOME:-$HOME/.local/share/sentia-dev-signing}"
GNUPG_HOME="$SIGNING_HOME/gnupg"
PUBLIC_DIR="$SIGNING_HOME/public"
DEV_UID="${SENTIA_DEV_UID:-Sentia Development Archive Signing <dev-signing@sentia.local>}"

umask 077
mkdir -p "$SIGNING_HOME" "$GNUPG_HOME" "$PUBLIC_DIR"
chmod 700 "$SIGNING_HOME" "$GNUPG_HOME" "$PUBLIC_DIR"

export GNUPGHOME="$GNUPG_HOME"

if ! gpg --batch --list-keys "$DEV_UID" >/dev/null 2>&1; then
  cat > "$SIGNING_HOME/dev-key.batch" <<'BATCH'
%no-protection
Key-Type: eddsa
Key-Curve: ed25519
Key-Usage: sign
Name-Real: Sentia Development Archive Signing
Name-Email: dev-signing@sentia.local
Expire-Date: 1y
%commit
BATCH

  gpg --batch --generate-key "$SIGNING_HOME/dev-key.batch"
  rm -f "$SIGNING_HOME/dev-key.batch"
fi

fingerprint="$({ gpg --batch --with-colons --list-keys "$DEV_UID" || true; } | awk -F: '/^fpr:/{print $10; exit}')"
if [[ -z "$fingerprint" ]]; then
  echo "failed to resolve development signing fingerprint" >&2
  exit 1
fi

printf '%s\n' "$fingerprint" > "$SIGNING_HOME/dev-key.fingerprint"
gpg --batch --yes --export "$fingerprint" > "$PUBLIC_DIR/sentia-archive-keyring.gpg"
chmod 0644 "$PUBLIC_DIR/sentia-archive-keyring.gpg"

echo "$fingerprint"
