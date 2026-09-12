# Acceptance

Canonical machine-readable manifest: `tests/acceptance/manifest.yaml`

## Current status

No acceptance family is passed at this checkpoint. Every family is planned and
remains `not-run`.

## Acceptance families

| ID | Meaning | Current status |
| --- | --- | --- |
| BUILD | Clean documented build, manifests/checksums, license/source closure, and ISO validity. | not-run |
| LIVE | Firmware boot, userspace, login, desktop, terminal, router, local model, health, and installer launch. | not-run |
| INSTALL | Actual Calamares automatic/manual GPT installs with offline handoff and no live residue. | not-run |
| DISK | ISO removed, installed-system boot, network, updates, local AI, terminal help, and reboot/shutdown. | not-run |
| OFFLINE | Guest NIC disconnected, local question answered, first boot skippable, no hidden model download. | not-run |
| AI | Exact model identity, baseline/optimized CPU builds, bounded queue and memory, cancellation, and isolation. | not-run |
| TOOLS | Structured registry, schemas, provenance, permission handling, and safe package-plan workflow. | not-run |
| SHELL | Bash preservation, classification, bounded capture, and no automatic command execution. | not-run |
| HEALTH | Shared metrics, missing-sensor handling, sustained-condition detection, and no automatic remediation. | not-run |
| ROUTER | Four policies, startup/ready/fallback states, and clean midstream fallback. | not-run |
| PRIVACY | No secret leakage, exact consent, protected sockets, and denied access to forbidden provider state. | not-run |
| TRUST | Valid signatures, allowlist enforcement, and rejection of tampered or unsigned sources. | not-run |
| SECUREBOOT | OVMF boot-chain outcomes recorded without claiming unsupported physical-hardware certification. | not-run |

## Rule

Any future machine-generated acceptance result must be published back to the
manifest and this document together so they do not drift.
