#!/usr/bin/env bash
set -euo pipefail

SENTIA_PACKAGING_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SENTIA_REPO_ROOT="$(cd "$SENTIA_PACKAGING_LIB_DIR/../.." && pwd)"
SENTIA_HEAVY_LOCK="${SENTIA_HEAVY_LOCK:-/home/ubuntu/sentia-linux/artifacts/.locks/heavy.lock}"

require_tool() {
  local tool="$1"
  command -v "$tool" >/dev/null 2>&1 || {
    echo "missing required tool: $tool" >&2
    return 1
  }
}

acquire_heavy_lock() {
  mkdir -p "$(dirname "$SENTIA_HEAVY_LOCK")"
  exec 9>"$SENTIA_HEAVY_LOCK"
  flock 9
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
