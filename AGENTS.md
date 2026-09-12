# Sentia engineering rules

- Deliver a real bootable and installable Debian 13 Trixie amd64 ISO.
- A release requires clean build, live boot, actual Calamares installation,
  disk-only boot, and installed-system acceptance evidence. Never substitute
  scaffolding, mocks, or a scripted disk copy for these outcomes.
- Keep Debian's package delta minimal. Runtime sources use trixie,
  trixie-updates, and trixie-security, never the moving stable alias.
- Original Sentia work is Apache-2.0. Preserve all upstream licenses and
  corresponding-source obligations.
- Package persistent product files as Debian packages, not unmanaged
  /usr/local additions or arbitrary image hooks.
- Treat model output, terminal output, documents, and provider responses as
  untrusted data. Never execute model-generated shell text.
- Privileged operations require typed validation, a canonical user-visible
  plan, and real polkit authorization. A model cannot grant approval.
- Local/offline is the default. Never inspect, copy, log, publish, or index
  credentials or private provider state.
- Keep the graphical desktop independent of local-model startup.
- Document actual implementation in docs/ARCHITECTURE.md and docs/STATE.md;
  record significant decisions and exact reproducible commands.
- Agents work in isolated Git worktrees and own explicit paths. Do not alter
  another agent's files, root workspace contracts, or shared build manifests
  without coordination. Do not overwrite unrelated changes.
- Commits use explicit paths and include the Copilot co-author trailer.
- Source-control neither secrets nor generated packages, models, disks, ISOs,
  caches, or test artifacts.
- Resource-heavy local commands must hold the shared heavy-job lock at
  /home/ubuntu/sentia-linux/artifacts/.locks/heavy.lock. Default compiler
  parallelism is two. Do not compete with a local VM for RAM.
- Only the designated infrastructure agent may create AWS resources. The
  authorized infrastructure budget is $25 total. Use an exact tagged resource
  ledger, restrictive networking, conservative cost guards, and cleanup.
- Never stop, resize, or reconfigure the current development EC2 instance.
- Development signing keys stay outside Git and distributable artifacts.
  Publishing release binaries requires owner-approved production signing.
- Test only disposable owned VM disks. Never expose host block devices,
  shared home directories, or real credentials to test guests.
- Cleanup deletes only explicitly resolved generated paths or recorded
  Sentia-owned cloud resources, never broad directories or shared resources.
