# Sentia builder workflow (Debian 13 Trixie)

## Scope

This document describes the builder-owned orchestration for:

- `make bootstrap`
- `make packages`
- `make repo`
- `make iso`
- `make test`
- `make test-iso`
- `make test-install`
- `make test-failure`
- `make clean`

The builder uses a Debian 13 (`trixie`) root at:

- `/home/ubuntu/sentia-linux/.build/chroots/debian-trixie-amd64`

Shared heavy-job lock:

- `/home/ubuntu/sentia-linux/artifacts/.locks/heavy.lock`

Builder artifacts/logs/manifests:

- `/home/ubuntu/sentia-linux/artifacts/builder/`

## Dependency manifests

Host dependencies are declared in:

- `build/manifests/host-dependencies.txt`

Builder (chroot) dependencies are declared in:

- `build/manifests/builder-dependencies.txt`

Debian runtime sources (Deb822) are declared in:

- `build/manifests/debian-trixie.sources`

Suites are pinned explicitly to:

- `trixie`
- `trixie-updates`
- `trixie-security`

## Exact bootstrap command

Run from repo root:

```bash
make bootstrap
```

`make bootstrap` performs:

1. Host `apt-get update/install` from `host-dependencies.txt` under `flock`.
2. `debootstrap` of Debian 13 Trixie with `main non-free-firmware`.
3. Deb822 source installation in the chroot.
4. Builder dependency install inside `systemd-nspawn`.
5. Manifest capture for:
   - host packages
   - builder packages
   - apt index checksums
   - host/builder rust/cargo paths and versions
   - input source checksums and `SOURCE_DATE_EPOCH`

Idempotent manifest refresh (no package reinstall):

```bash
SENTIA_SKIP_HOST_APT=1 SENTIA_SKIP_BUILDER_APT=1 make bootstrap
```

## Integration directories (stable contracts)

- `build/input/packages/pool/` (package agent output)
- `build/input/signing/public/sentia-archive-keyring.gpg` (signing agent output)
- `config/live-build/package-lists/` (desktop/live-build agent-owned input)
- `tests/vm/test-install.sh` (VM installer automation harness)

## Build commands

```bash
make packages
make repo
make iso
make test-failure
make test-iso
make test-install
make test
```

Fail-closed behavior is required:

- Missing package inputs -> `make packages` fails.
- Missing signing key ID or keyring input -> `make repo` fails.
- Missing live-build package lists -> `make iso` fails.
- Missing installer automation harness -> `make test-install` fails.

## Reproducibility and provenance

- `SOURCE_DATE_EPOCH` comes from the latest git commit timestamp.
- `config/live-build/auto/build` enforces `lb build --build-with-chroot true`.
- Atomic write pattern (`*.tmp.$$` then `mv`) is used for manifests.
- Current apt index checksums are captured after bootstrap.
- Builder container operations export `MAKEFLAGS=-j2`.

## Rust availability

Bootstrap records Rust toolchain location/version in:

- `/home/ubuntu/sentia-linux/artifacts/builder/manifests/host-rust.txt`
- `/home/ubuntu/sentia-linux/artifacts/builder/manifests/builder-rust.txt`

## Cleanup

```bash
make clean
```

Removes only builder-owned generated paths under `.build/work` and
`artifacts/builder/*`.

Optional full bootstrap reset (including chroot):

```bash
./build/scripts/clean.sh --all
```
