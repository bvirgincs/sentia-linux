# State

```yaml
checkpoint: ai-in-the-image
source: implemented
packages: building
iso: building
live-boot: partial
install: not-tested
release: not-qualified
```

Updated after the first live-boot probe of an ISO that contains the AI stack.

## Working now, with evidence

- The full package set builds inside the Debian 13 nspawn builder: base-files,
  keyring, core, desktop, the six Rust packages, `sentia-llama-cpp` and the
  2.14 GB `sentia-granite-model`.
- `sentia-llama-cpp` ships `/usr/bin/llama-server`, its private
  `libllama-server-impl.so` and fourteen dynamically loaded CPU backends from
  SSE4.2 through Sapphire Rapids, so one binary covers baseline and modern CPUs.
- Granite answers locally over the private Unix socket, CPU-only, at roughly
  15.8 tokens/second, with populated `content`.
- The ISO builds at about 3.6 GB with a zstd-compressed squashfs and contains
  every Sentia package, the runtime and the model weights.
- The ISO boots under KVM and reaches a login prompt. Passing live checks:
  Sentia identity and os-release ownership, hostname, no failed units, LightDM,
  Xorg, the Xfce session file, Chromium running, Thunar, Calamares, literal
  Debian suites, the signed offline archive, `Signed-By` trust, the AI runtime
  binary and model bytes, the runtime and broker services, private socket
  permissions and no externally bound listener.

## Known failures

Nothing below is deferred work; each has a fix in flight or already committed.

- The per-user router units were installed as system units, so no router socket
  existed. Fixed; awaiting a rebuild to confirm.
- The live session stopped at a LightDM greeter, so there was no user session
  and therefore no user systemd manager. Autologin added to `sentia-live`;
  awaiting a rebuild to confirm.
- The offline-answer check never ran because the guest check script could not
  find `runuser`. Fixed.
- `sentia-providers` `adapter::tests::responses_match_reviewed_fixtures` fails.

## Not tested yet

- Calamares installation to a virtual disk, boot from that disk with the ISO
  removed, and every installed-system acceptance check.
- The failure-injection matrix and the independent security review.
- Secure Boot behaviour under OVMF.
- Any remote provider against a real account. No credentials have been supplied
  and none are required for the offline product claim.

## Blocked or pending

- Production signing has not been requested or granted, so no release may be
  published. The development key is local to the build host and is not in Git
  or in any artifact.
