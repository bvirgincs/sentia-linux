# State

```yaml
checkpoint: foundation
source: initial-source-only
runtime: not-tested
integration: not-tested
release: not-qualified
```

## Working now

- Repository identity and mission text.
- Licensing and notice files for the foundation checkpoint.
- Developer-facing README guidance for the planned `make` interface.
- Honest state and decision documentation.
- Custom agent profiles under `.github/agents/`.
- A machine-readable acceptance manifest that remains unrun.

## Untested

- Runtime, routing, tools, desktop, installer, package, and provider code.
- Live boot, Calamares, disk-only boot, installed-system, and failure-injection
  evidence.
- Any AWS runner or production-signing path.

## Blocked or pending

- Builder and package manifests are not part of this checkpoint.
- No release candidate can be claimed from the current source set.
- No private credentials, account IDs, or private hostnames are recorded here.

## Honest summary

This worktree establishes the foundation docs only. It does not yet validate
the runtime, integration, installation, or release gates described in the
plan.
