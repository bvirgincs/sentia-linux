# Sentia Linux

**Linux that understands what you're trying to do.**

Sentia is an in-development Debian 13 Trixie derivative for amd64, with a
minimal Xfce desktop and local, offline AI assistance.

## Current checkpoint

This worktree is a foundation-only checkpoint. It contains repository identity,
documentation, and custom agent profiles. It does not yet contain the build
system, schemas, packages, runtime, installer, or a qualified release.

No runtime, integration, live-boot, Calamares, or production-signing evidence
exists here yet.

The canonical acceptance manifest lives in
`tests/acceptance/manifest.json` under contract-engineer ownership and is
pending integration. This checkpoint does not implement the acceptance schema.

## Repository boundaries

- Original Sentia work is Apache-2.0.
- Upstream-derived components retain their own licenses and source obligations.
- The GPL-linked APT worker and Debian-derived components must remain under
  their compatible upstream terms.

## Developer entrypoints

The planned builder interface, once the builder merge lands, is:

- `make bootstrap`
- `make packages`
- `make repo`
- `make iso`
- `make test`
- `make test-iso`
- `make test-install`
- `make test-failure`
- `make release`
- `make clean`

These targets are the anticipated project interface and are not yet wired in
this worktree.

## Read next

- `docs/ARCHITECTURE.md`
- `docs/STATE.md`
- `docs/ACCEPTANCE.md`
- `docs/TESTING.md`
- `docs/PROVIDERS.md`
- `docs/SECURITY.md`
- `docs/RELEASE.md`
- `docs/DECISIONS.md`
