# Architecture

## Status at this checkpoint

- Implemented: repository foundation
  - `README.md`
  - `LICENSE`
  - `NOTICE`
  - `docs/*`
  - `.github/agents/*.agent.md`
- Implemented: build-host artifact integrity tools under `build/release/`,
  with focused tests under `tests/release/`. These use Python's standard
  library on the build host, not a Python dependency in Sentia's AI runtime.
- Pending outside this scope: the canonical acceptance manifest
  `tests/acceptance/manifest.json`, owned by the contract engineer.
- Not yet integrated: build system, schemas, packages, runtime, installer,
  acceptance tests, VM automation, or signed release publication.
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

Independent components are developed concurrently. Only the foundation and
artifact integrity utility are currently integrated in this checkpoint.

## Boundaries

- Artifact code is implemented; no operating-system packages are qualified.
- No secrets, account IDs, or private hostnames are recorded here.
- No claim is made that the runtime, installer, or release pipeline exists yet.

## Reproducible commands

```sh
python3 build/release/release_artifacts.py --help
python3 -m unittest discover -s tests/release -p 'test_*.py' -v
```

The ISO builder interface is pending integration.
