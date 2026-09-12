# tests/apt

Test policy:

- No host APT mutation tests are run here.
- Mutation/transaction tests must run only inside disposable owned Debian
  chroot/VM artifacts with test packages.
- Local CI in this worktree runs read-only and contract-level checks.
