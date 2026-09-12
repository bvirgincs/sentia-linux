#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
SUITE="${SENTIA_ARCHIVE_SUITE:-sentia-0.1}"
INPUT_DIR="${SENTIA_PACKAGE_INPUT_DIR:-$REPO_ROOT/artifacts/packages}"
OUTPUT_ROOT="${SENTIA_REPOSITORY_OUTPUT_DIR:-$REPO_ROOT/artifacts/repository}"
ALLOWLIST_FILE="$REPO_ROOT/config/apt/sentia-allowed-binaries.txt"
SOURCE_ALLOWLIST_FILE="$REPO_ROOT/build/repository/allowed-source-packages.txt"
SIGNING_KEY="${SENTIA_SIGNING_KEY_FPR:-}"
PUBLISH_MODE="${SENTIA_PUBLISH_MODE:-development}"

if [[ -z "$SIGNING_KEY" ]]; then
  echo "SENTIA_SIGNING_KEY_FPR is required" >&2
  exit 1
fi

"$REPO_ROOT/build/signing/require-approved-signing-key.sh" "$SIGNING_KEY" "$PUBLISH_MODE"

for tool in reprepro dpkg-deb gpg; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "missing required tool: $tool" >&2
    exit 1
  }
done

if [[ ! -d "$INPUT_DIR" ]]; then
  echo "package input directory not found: $INPUT_DIR" >&2
  exit 1
fi

mapfile -t allowed_binaries < <(grep -Ev '^\s*(#|$)' "$ALLOWLIST_FILE")
declare -A allowed_map=()
for pkg in "${allowed_binaries[@]}"; do
  allowed_map["$pkg"]=1
done

mapfile -t allowed_sources < <(grep -Ev '^\s*(#|$)' "$SOURCE_ALLOWLIST_FILE")
declare -A source_allowed_map=()
for src in "${allowed_sources[@]}"; do
  source_allowed_map["$src"]=1
done

mapfile -t debs < <(find "$INPUT_DIR" -type f -name '*.deb' | sort)
if [[ ${#debs[@]} -eq 0 ]]; then
  echo "no .deb files found under $INPUT_DIR" >&2
  exit 1
fi

compact_debs=()
complete_manifest="$OUTPUT_ROOT/manifests/complete-packages.txt"
compact_manifest="$OUTPUT_ROOT/manifests/compact-packages.txt"
mkdir -p "$(dirname "$complete_manifest")"
: > "$complete_manifest"
: > "$compact_manifest"

for deb in "${debs[@]}"; do
  pkg="$(dpkg-deb -f "$deb" Package)"
  ver="$(dpkg-deb -f "$deb" Version)"
  size_bytes="$(stat -c '%s' "$deb")"
  if [[ -z "$pkg" ]]; then
    echo "failed to read package name from: $deb" >&2
    exit 1
  fi

  if [[ -z "${allowed_map[$pkg]:-}" ]]; then
    echo "package not in Sentia allowlist: $pkg ($deb)" >&2
    exit 1
  fi

  if [[ "$pkg" != base-files && "$pkg" != sentia-* ]]; then
    echo "unexpected non-Sentia namespace package rejected: $pkg" >&2
    exit 1
  fi

  printf '%s %s %s\n' "$pkg" "$ver" "$deb" >> "$complete_manifest"

  if [[ "$pkg" != "sentia-granite-model" && "$pkg" != "sentia-offline-repository" && "$size_bytes" -lt 734003200 ]]; then
    compact_debs+=("$deb")
    printf '%s %s %s\n' "$pkg" "$ver" "$deb" >> "$compact_manifest"
  fi
done

mapfile -t dsc_files < <(find "$INPUT_DIR" -type f -name '*.dsc' | sort)

init_repo() {
  local base_dir="$1"
  rm -rf "$base_dir"
  mkdir -p "$base_dir/conf"

  cat > "$base_dir/conf/distributions" <<DIST
Origin: Sentia
Label: Sentia
Suite: $SUITE
Codename: $SUITE
Architectures: amd64 source
Components: main
Description: Sentia signed package overlay
SignWith: $SIGNING_KEY
DebIndices: Packages Release . .gz
DscIndices: Sources Release . .gz
DIST
}

include_dsc_if_allowed() {
  local repo_dir="$1"
  local dsc="$2"
  local src
  src="$(awk -F': ' '/^Source:/{print $2; exit}' "$dsc")"
  if [[ -z "$src" ]]; then
    src="$(basename "$dsc" | sed -E 's/_.*$//')"
  fi
  if [[ -n "${source_allowed_map[$src]:-}" ]]; then
    reprepro --basedir "$repo_dir" includedsc "$SUITE" "$dsc"
  fi
}

populate_repo() {
  local repo_dir="$1"
  shift
  local list=("$@")

  for deb in "${list[@]}"; do
    reprepro --basedir "$repo_dir" includedeb "$SUITE" "$deb"
  done

  for dsc in "${dsc_files[@]}"; do
    include_dsc_if_allowed "$repo_dir" "$dsc"
  done

  reprepro --basedir "$repo_dir" export "$SUITE"

  for required in \
    "dists/$SUITE/Release" \
    "dists/$SUITE/InRelease" \
    "dists/$SUITE/Release.gpg" \
    "dists/$SUITE/main/source/Sources" \
    "dists/$SUITE/main/binary-amd64/Packages"; do
    if [[ ! -f "$repo_dir/$required" ]]; then
      echo "repository missing required metadata: $repo_dir/$required" >&2
      exit 1
    fi
  done
}

COMPLETE_DIR="$OUTPUT_ROOT/complete"
COMPACT_DIR="$OUTPUT_ROOT/compact"

init_repo "$COMPLETE_DIR"
populate_repo "$COMPLETE_DIR" "${debs[@]}"

if [[ ${#compact_debs[@]} -eq 0 ]]; then
  echo "no packages left for compact archive after exclusions" >&2
  exit 1
fi

init_repo "$COMPACT_DIR"
populate_repo "$COMPACT_DIR" "${compact_debs[@]}"

if find "$COMPACT_DIR/pool" -type f -size +700M | grep -q .; then
  echo "compact repository contains oversized package payloads" >&2
  exit 1
fi

echo "complete archive: $COMPLETE_DIR"
echo "compact archive: $COMPACT_DIR"
