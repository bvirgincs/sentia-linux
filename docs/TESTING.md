# Testing

## Current checkpoint

No runtime, integration, VM, or installer tests have been executed in this
worktree yet. The artifact integrity utility has an executable focused suite:

```sh
python3 -m unittest discover -s tests/release -p 'test_*.py' -v
```

Seventeen tests currently pass. They establish byte-integrity and failure
handling, not that an ISO boots or installs.

## Planned validation interface

The intended builder and release interface is:

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

These commands are the planned project contract. They are not yet wired in this
worktree and must not be reported as completed here.

## Evidence rules

- Record the exact command, input artifacts, environment, and outputs for every
  future test run.
- Keep evidence under ignored `artifacts/<build-id>/` paths.
- Distinguish planned validation from executed validation.

## Checkpoint validation already run

- `git diff --check`
- `python3` with PyYAML to parse every `.github/agents/*.agent.md` frontmatter
  block
- The artifact integrity suite above, including collision/non-clobber,
  malformed and oversized manifest, FIFO rejection, and cleanup-failure cases

## Acceptance manifest status

The canonical JSON manifest is owned by the contract engineer and is pending
integration, so no acceptance-manifest validation has run in this checkpoint.
