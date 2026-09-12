#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
WORK_DIR="$REPO_ROOT/.build/tests/packaging/apt-trust"
CONFIG_DIR="$REPO_ROOT/config/apt"

rm -rf "$WORK_DIR"
mkdir -p "$WORK_DIR"/{keys,debs,repos}

create_key() {
  local homedir="$1"
  local name="$2"
  local email="$3"
  mkdir -p "$homedir"
  chmod 700 "$homedir"

  cat > "$homedir/key.batch" <<BATCH
%no-protection
Key-Type: eddsa
Key-Curve: ed25519
Key-Usage: sign
Name-Real: $name
Name-Email: $email
Expire-Date: 1y
%commit
BATCH

  GNUPGHOME="$homedir" gpg --batch --generate-key "$homedir/key.batch" >/dev/null 2>&1
  rm -f "$homedir/key.batch"
  GNUPGHOME="$homedir" gpg --batch --with-colons --list-keys "$email" | awk -F: '/^fpr:/{print $10; exit}'
}

build_deb() {
  local pkg="$1"
  local ver="$2"
  local out_dir="$3"
  local pkg_dir="$WORK_DIR/pkg-${pkg}-${ver}"
  local deb_path="$out_dir/${pkg}_${ver}_amd64.deb"

  rm -rf "$pkg_dir"
  mkdir -p "$pkg_dir/DEBIAN" "$pkg_dir/usr/share/$pkg"

  cat > "$pkg_dir/DEBIAN/control" <<CONTROL
Package: $pkg
Version: $ver
Section: misc
Priority: optional
Architecture: amd64
Maintainer: Sentia Packaging Tests <packaging-tests@sentia.invalid>
Description: test package $pkg
CONTROL

  printf '%s\n' "$pkg $ver" > "$pkg_dir/usr/share/$pkg/version"
  dpkg-deb --root-owner-group --build "$pkg_dir" "$deb_path" >/dev/null
  printf '%s\n' "$deb_path"
}

build_repo() {
  local repo_dir="$1"
  local suite="$2"
  local origin="$3"
  local label="$4"
  local key_home="$5"
  local key_fpr="$6"
  shift 6
  local debs=("$@")

  rm -rf "$repo_dir"
  mkdir -p "$repo_dir/pool/main" "$repo_dir/dists/$suite/main/binary-amd64"

  for deb in "${debs[@]}"; do
    cp -a "$deb" "$repo_dir/pool/main/"
  done

  apt-ftparchive packages "$repo_dir/pool/main" > "$repo_dir/dists/$suite/main/binary-amd64/Packages"
  gzip -n9c "$repo_dir/dists/$suite/main/binary-amd64/Packages" > "$repo_dir/dists/$suite/main/binary-amd64/Packages.gz"

  cat > "$repo_dir/release.conf" <<REL
APT::FTPArchive::Release::Origin "$origin";
APT::FTPArchive::Release::Label "$label";
APT::FTPArchive::Release::Suite "$suite";
APT::FTPArchive::Release::Codename "$suite";
APT::FTPArchive::Release::Architectures "amd64";
APT::FTPArchive::Release::Components "main";
REL

  apt-ftparchive -c "$repo_dir/release.conf" release "$repo_dir/dists/$suite" > "$repo_dir/dists/$suite/Release"
  GNUPGHOME="$key_home" gpg --batch --yes --pinentry-mode loopback --default-key "$key_fpr" --output "$repo_dir/dists/$suite/Release.gpg" --detach-sign "$repo_dir/dists/$suite/Release"
  GNUPGHOME="$key_home" gpg --batch --yes --pinentry-mode loopback --default-key "$key_fpr" --output "$repo_dir/dists/$suite/InRelease" --clearsign "$repo_dir/dists/$suite/Release"
}

setup_apt_root() {
  local apt_root="$1"
  local debian_repo="$2"
  local sentia_repo="$3"
  local debian_keyring="$4"
  local sentia_keyring="$5"

  rm -rf "$apt_root"
  mkdir -p "$apt_root"/{etc/apt/sources.list.d,etc/apt/preferences.d,etc/apt/apt.conf.d,var/lib/apt/lists/partial,var/cache/apt/archives/partial,var/lib/dpkg,usr/share/keyrings}
  touch "$apt_root/var/lib/dpkg/status"

  cp -a "$debian_keyring" "$apt_root/usr/share/keyrings/debian-archive-keyring.gpg"
  cp -a "$sentia_keyring" "$apt_root/usr/share/keyrings/sentia-archive-keyring.gpg"
  cp -a "$CONFIG_DIR/50-sentia-origin.pref" "$apt_root/etc/apt/preferences.d/50-sentia-origin.pref"

  cat > "$apt_root/etc/apt/sources.list.d/debian.sources" <<EOF_DEB
Types: deb
URIs: file:$debian_repo
Suites: trixie
Components: main
Signed-By: $apt_root/usr/share/keyrings/debian-archive-keyring.gpg
EOF_DEB

  cat > "$apt_root/etc/apt/sources.list.d/sentia.sources" <<EOF_SENTIA
Types: deb
URIs: file:$sentia_repo
Suites: sentia-0.1
Components: main
Signed-By: $apt_root/usr/share/keyrings/sentia-archive-keyring.gpg
EOF_SENTIA

  cat > "$apt_root/etc/apt/apt.conf" <<EOF_APT
Dir "$apt_root";
Dir::Etc "etc/apt";
Dir::State "var/lib/apt";
Dir::Cache "var/cache/apt";
Dir::State::status "var/lib/dpkg/status";
APT::Architecture "amd64";
Debug::NoLocking "true";
Acquire::AllowInsecureRepositories "false";
Acquire::AllowDowngradeToInsecureRepositories "false";
EOF_APT
}

run_apt() {
  local apt_root="$1"
  shift
  APT_CONFIG="$apt_root/etc/apt/apt.conf" "$@"
}

DEBIAN_KEY_HOME="$WORK_DIR/keys/debian"
SENTIA_KEY_HOME="$WORK_DIR/keys/sentia"
DEBIAN_KEY_FPR="$(create_key "$DEBIAN_KEY_HOME" "Debian Test Archive" "debian-test@example.invalid")"
SENTIA_KEY_FPR="$(create_key "$SENTIA_KEY_HOME" "Sentia Test Archive" "sentia-test@example.invalid")"

GNUPGHOME="$DEBIAN_KEY_HOME" gpg --batch --yes --export "$DEBIAN_KEY_FPR" > "$WORK_DIR/keys/debian-archive-keyring.gpg"
GNUPGHOME="$SENTIA_KEY_HOME" gpg --batch --yes --export "$SENTIA_KEY_FPR" > "$WORK_DIR/keys/sentia-archive-keyring.gpg"

DEBIAN_COREUTILS_DEB="$(build_deb coreutils 1.0-1 "$WORK_DIR/debs")"
SENTIA_COREUTILS_DEB="$(build_deb coreutils 9.9-1 "$WORK_DIR/debs")"
SENTIA_BASE_DEB="$(build_deb sentia-base 0.1-1 "$WORK_DIR/debs")"

DEBIAN_REPO="$WORK_DIR/repos/debian"
SENTIA_REPO="$WORK_DIR/repos/sentia"

build_repo "$DEBIAN_REPO" trixie Debian Debian "$DEBIAN_KEY_HOME" "$DEBIAN_KEY_FPR" "$DEBIAN_COREUTILS_DEB"
build_repo "$SENTIA_REPO" sentia-0.1 Sentia Sentia "$SENTIA_KEY_HOME" "$SENTIA_KEY_FPR" "$SENTIA_COREUTILS_DEB" "$SENTIA_BASE_DEB"

APT_ROOT_GOOD="$WORK_DIR/apt-good"
setup_apt_root "$APT_ROOT_GOOD" "$DEBIAN_REPO" "$SENTIA_REPO" "$WORK_DIR/keys/debian-archive-keyring.gpg" "$WORK_DIR/keys/sentia-archive-keyring.gpg"

run_apt "$APT_ROOT_GOOD" apt-get update >/dev/null

coreutils_policy="$(run_apt "$APT_ROOT_GOOD" apt-cache policy coreutils)"
coreutils_candidate="$(awk '/Candidate:/ {print $2; exit}' <<<"$coreutils_policy")"
if [[ "$coreutils_candidate" != "1.0-1" ]]; then
  echo "unexpected coreutils candidate (pin policy takeover regression): $coreutils_candidate" >&2
  exit 1
fi

sentia_base_policy="$(run_apt "$APT_ROOT_GOOD" apt-cache policy sentia-base)"
sentia_base_candidate="$(awk '/Candidate:/ {print $2; exit}' <<<"$sentia_base_policy")"
if [[ "$sentia_base_candidate" != "0.1-1" ]]; then
  echo "unexpected sentia-base candidate (allowlist regression): $sentia_base_candidate" >&2
  exit 1
fi

APT_ROOT_TAMPERED="$WORK_DIR/apt-tampered"
setup_apt_root "$APT_ROOT_TAMPERED" "$DEBIAN_REPO" "$SENTIA_REPO" "$WORK_DIR/keys/debian-archive-keyring.gpg" "$WORK_DIR/keys/sentia-archive-keyring.gpg"
printf '#tamper\n' >> "$SENTIA_REPO/dists/sentia-0.1/InRelease"
if run_apt "$APT_ROOT_TAMPERED" apt-get update >/dev/null 2>&1; then
  echo "tampered signed repository unexpectedly accepted" >&2
  exit 1
fi

SENTIA_UNSIGNED_REPO="$WORK_DIR/repos/sentia-unsigned"
cp -a "$SENTIA_REPO" "$SENTIA_UNSIGNED_REPO"
rm -f "$SENTIA_UNSIGNED_REPO/dists/sentia-0.1/InRelease" "$SENTIA_UNSIGNED_REPO/dists/sentia-0.1/Release.gpg"
APT_ROOT_UNSIGNED="$WORK_DIR/apt-unsigned"
setup_apt_root "$APT_ROOT_UNSIGNED" "$DEBIAN_REPO" "$SENTIA_UNSIGNED_REPO" "$WORK_DIR/keys/debian-archive-keyring.gpg" "$WORK_DIR/keys/sentia-archive-keyring.gpg"
if run_apt "$APT_ROOT_UNSIGNED" apt-get update >/dev/null 2>&1; then
  echo "unsigned repository unexpectedly accepted" >&2
  exit 1
fi
