# Release

## Status

Not release-qualified.

## Required gates

Any future release must have:

- clean build evidence
- live boot evidence
- actual Calamares installation evidence
- ISO-removed installed-system boot evidence
- installed-system acceptance and failure-injection evidence
- source closure, checksums, and signed release material
- owner-approved production signing before publishable binaries are released

## Boundaries

- Development signing may exist later, but no private key belongs in Git or
  distributable artifacts.
- Production signing remains a hard gate.
- Oversized release assets must be split deterministically and reconstructed
  before publication.
- The AWS test-runner budget remains capped at the approved total.

## Stop condition

Do not mark a release complete until the manifest, the evidence, and the
documentation all agree.
