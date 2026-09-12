# State

```yaml
checkpoint: foundation-and-artifact-tools
source: partial-implementation
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
- Streaming release artifact splitting, verification, and exact reconstruction.
  Seventeen focused tests pass, including corruption, non-clobbering output,
  bounded manifest reads, and cleanup-failure handling.
- A pending reference to the contract-engineer-owned canonical acceptance
  manifest; no local acceptance schema is implemented here.

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

The merged source contains foundation documents and build-host artifact
utilities. The runtime, installer, and image pipeline are being implemented in
isolated worktrees and have not yet passed the release gates.
