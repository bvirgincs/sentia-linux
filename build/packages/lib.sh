#!/usr/bin/env bash
set -euo pipefail

SENTIA_PACKAGING_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SENTIA_REPO_ROOT="$(cd "$SENTIA_PACKAGING_LIB_DIR/../.." && pwd)"
SENTIA_HEAVY_LOCK="${SENTIA_HEAVY_LOCK:-$SENTIA_REPO_ROOT/artifacts/.locks/heavy.lock}"

require_tool() {
  local tool="$1"
  command -v "$tool" >/dev/null 2>&1 || {
    echo "missing required tool: $tool" >&2
    return 1
  }
}

# The lock is held on file descriptor 9 for as long as the descriptor is open,
# which for a top-level build script means until it exits. A script that goes on
# to invoke another builder that takes the same lock must call
# release_heavy_lock first, or it will wait forever for itself.
acquire_heavy_lock() {
  mkdir -p "$(dirname "$SENTIA_HEAVY_LOCK")"
  exec 9>"$SENTIA_HEAVY_LOCK"
  # Bounded rather than indefinite: a lock that is never granted is a bug, and a
  # build that hangs silently for hours is much harder to diagnose than one that
  # stops here and says so.
  if ! flock -w "${SENTIA_HEAVY_LOCK_TIMEOUT:-7200}" 9; then
    echo "timed out waiting for the heavy build lock: $SENTIA_HEAVY_LOCK" >&2
    exit 1
  fi
}

release_heavy_lock() {
  exec 9>&-
}

run_dpkg_buildpackage() {
  local source_dir="$1"
  local mode="$2"
  local -a extra_flags=()

  require_tool dpkg-buildpackage

  export DEB_BUILD_OPTIONS="${DEB_BUILD_OPTIONS:-parallel=2}"
  if [[ "${SENTIA_DPKG_IGNORE_BUILD_DEPS:-0}" == "1" ]]; then
    extra_flags+=("-d")
  fi

  case "$mode" in
    binary)
      (cd "$source_dir" && dpkg-buildpackage "${extra_flags[@]}" -us -uc -b -j2)
      ;;
    source)
      (cd "$source_dir" && dpkg-buildpackage "${extra_flags[@]}" -us -uc -S)
      ;;
    both)
      (cd "$source_dir" && dpkg-buildpackage "${extra_flags[@]}" -us -uc -S)
      (cd "$source_dir" && dpkg-buildpackage "${extra_flags[@]}" -us -uc -b -j2)
      ;;
    *)
      echo "invalid build mode: $mode" >&2
      return 1
      ;;
  esac
}

copy_deb_outputs() {
  local search_root="$1"
  local output_dir="$2"
  mkdir -p "$output_dir"

  find "$search_root" -maxdepth 2 -type f \( \
    -name '*.deb' -o -name '*.ddeb' -o -name '*.udeb' -o \
    -name '*.changes' -o -name '*.buildinfo' -o -name '*.dsc' -o \
    -name '*.tar.xz' -o -name '*.tar.gz' -o -name '*.debian.tar.xz' -o \
    -name '*.orig.tar.xz' \
  \) -exec cp -a {} "$output_dir/" \;
}
