# Architecture

## Status at this checkpoint

- Implemented: repository foundation only
  - `README.md`
  - `LICENSE`
  - `NOTICE`
  - `docs/*`
  - `.github/agents/*.agent.md`
- Pending outside this scope: the canonical acceptance manifest
  `tests/acceptance/manifest.json`, owned by the contract engineer.
- Not yet implemented: build system, schemas, packages, runtime, installer,
  tests, VM automation, or release automation.
- Not yet tested: runtime, integration, live boot, Calamares, installed-system,
  provider, or AWS paths.
- Release status: no qualified release.

## Target architecture

The approved plan still targets a layered Sentia system:

1. repository foundation and shared contracts
2. builder and Debian packaging
3. runtime, router, tools, and desktop integration
4. VM-driven boot/install evidence
5. source-complete release packaging and signing

That sequence is the intended delivery path, but only the foundation docs and
agent profiles exist in this checkpoint.

## Boundaries

- No code or packaging changes were made in this checkpoint.
- No secrets, account IDs, or private hostnames are recorded here.
- No claim is made that the runtime, installer, or release pipeline exists yet.

## Reproducible commands

No build or test commands are available from this checkpoint because the
builder contract has not yet been merged.
