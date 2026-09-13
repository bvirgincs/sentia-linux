# State

```yaml
checkpoint: live-ai-chain-repair
source: implemented
packages: built
iso: built-and-published
live-boot: partial
install: not-tested
release: not-qualified
```

Updated while repairing the local AI socket chain against a real KVM boot.

## The published ISO

`https://github.com/bvirgincs/sentia-linux/releases/tag/iso-20260913-0838`

`sentia-trixie-amd64.iso`, 3830726656 bytes, sha256
`338dd83bccfa6d21fcc4a9239ad1f059b9a496336a0f82ab45493587f5661fd7`, built from
commit `00dcc3b`. It is a pre-release: an artifact that exists and reassembles,
not a qualified Sentia 0.1. GitHub caps an asset at 2 GiB so it is published as
four parts with a manifest; the parts were downloaded back from the release and
rejoined to the exact original checksum.

A previous ISO was lost when its build runner self-terminated before anything
published it. Nothing is built now without being staged for publication.

## Working now, with evidence

- A clean host with only `build/manifests/builder-dependencies.txt` installed
  can build the whole ISO. This was not true before: four `-dev` packages were
  missing from the manifest and earlier builds only worked because the host had
  been modified by hand. `tests/build/test_builder_dependencies.py` now parses
  every `packaging/*/debian/control` and fails if the manifest cannot satisfy
  its `Build-Depends`.
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
- A runner with nested virtualisation (`m8i`, `NestedVirtualization=enabled` at
  launch) provides `/dev/kvm`, so `make test-iso` now runs against real
  hardware acceleration. EC2 offers nested virtualisation only on 8th
  generation Intel and only at launch; the earlier `m7i` runner could never
  have had it.
- The ISO boots under KVM and reaches a login prompt. Passing live checks:
  Sentia identity and os-release ownership, hostname, no failed units, LightDM,
  Xorg, the Xfce session file, Chromium running, Thunar, Calamares, literal
  Debian suites, the signed offline archive, `Signed-By` trust, the AI runtime
  binary and model bytes, the runtime and broker services, private socket
  permissions and no externally bound listener, `systemd` running, no failed
  system units and no failed user units, the graphical target active and
  Chromium reporting its version.

## Known failures

Nothing below is deferred work; each has a fix in flight or already committed.

The local AI chain is `ai` -> per-user router socket -> router -> broker socket
-> broker -> runtime socket -> llama.cpp. Every link had a defect that left both
systemd units `active (running)` while the chain was dead, and each was found
only by booting the image. In order of discovery:

- `sentia-router` called `listener_from_systemd()?.unwrap_or(bind_runtime_socket()?)`.
  `unwrap_or` evaluates its argument eagerly, so the fallback ran even when
  systemd had supplied the listener: it unlinked systemd's socket, bound its
  own, and then `unwrap_or` discarded and closed that second listener. `ss`
  still showed the router listening on the name through its inherited fd while
  every connection to the path was refused. Fixed with a lazy
  `acquire_listener`; `sentia-router.service` also lost its `[Install]` section,
  because a socket-activated service started from `default.target` is handed no
  listening fd and binds the path itself, giving the path two owners.
- The broker socket was bound `0660` owned by `sentia-inference`, a group no
  login account belongs to, so the router got `EACCES`. It is now `0666` and
  authorises callers from `SO_PEERCRED`, bounded at both ends by
  `minimum_peer_uid`/`maximum_peer_uid` so system and `DynamicUser` accounts are
  still refused.
- The runtime unit created its socket in `/run/sentia-local` at `0770`, while
  the broker, the router contract, the JSON schema and the shipped broker
  configuration all say `/run/sentia-inference` at `0600` in a `0700`
  directory. The broker would have refused the socket even if it had been at
  the expected path. Both units also declared `RuntimeDirectory=sentia-local`,
  so they disagreed about its mode and a runtime restart deleted the broker's
  socket. The runtime now owns `/run/sentia-inference`.

`live_checks.sh` gained `AI-BROKER-SOCKET`, an asserted rather than reported
`AI-RUNTIME-SOCKET`, and `AI-RUNTIME-READY`, so the next failure in this chain
is reported where it happens instead of as an unexplained `AI-OFFLINE-ANSWER`.

- The per-user router units were installed as system units, so no router socket
  existed. Fixed and present in the published ISO, but not yet re-probed.
- The live session stopped at a LightDM greeter, so there was no user session
  and therefore no user systemd manager. Autologin added to `sentia-live`.
  Present in the published ISO, not yet re-probed.
- The readiness probe was Python in a package with no `python3` dependency and
  no writable path under `ProtectSystem=strict`, so every boot was `degraded`.
  Rewritten as POSIX shell over `curl --unix-socket`. Not yet re-probed.
- `AI-OFFLINE-ANSWER` passed while its own output said
  `runuser: may not be used by non-root users`, because it searched output text
  instead of checking exit status. Now judged on exit status.

## Not tested yet

- Calamares installation to a virtual disk, boot from that disk with the ISO
  removed, and every installed-system acceptance check.
- The broker-to-runtime link. It has never executed: the chain has failed
  earlier on every boot so far.
- The failure-injection matrix and the independent security review.
- Secure Boot behaviour under OVMF.
- Any remote provider against a real account. No credentials have been supplied
  and none are required for the offline product claim.

## Blocked or pending

- Production signing has not been requested or granted, so no release may be
  published. The development key is local to the build host and is not in Git
  or in any artifact.
