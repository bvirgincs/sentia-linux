# Builder integration inputs

These directories are stable integration points consumed by `make` targets in
this worktree.

- `build/input/packages/pool/`: `.deb` packages from the package agent.
- `build/input/signing/public/`: public keyring input, including
  `sentia-archive-keyring.gpg`, from the signing agent.
- `config/live-build/package-lists/`: package list inputs from the desktop/live
  build agent (owned by that agent; builder does not modify it).
- `tests/vm/test-install.sh`: installer automation harness from the VM test
  agent.

Private signing keys must never be committed or copied into the builder chroot.
