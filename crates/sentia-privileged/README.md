# Sentia privileged boundary

This Apache-2.0 Rust 2021 executable owns `org.sentia.System1` on the **system**
bus, with object `/org/sentia/System1` and interface `org.sentia.System1`.
The executable connects to the fixed system socket, never an inherited
`DBUS_SYSTEM_BUS_ADDRESS`. Install it as
`/usr/libexec/sentia/sentia-privileged`, root-owned and not writable by users.

## Trusted frontend API

There are two methods:

* `Prepare(request: string) -> string`: validate versioned JSON, identify the
  bus sender, inspect current state, and return a canonical preview.
* `Apply(plan_id: string, digest: string) -> string`: consume that exact plan,
  revalidate, ask **polkit**, revalidate again, and execute the fixed operation.

`Prepare` accepts:

```json
{"version":1,"operation":{"operation":"service","action":"restart","unit":"ssh.service"}}
```

Other operation objects are:

```json
{"operation":"process_kill","pid":1234}
{"operation":"apt_install","packages":["curl"]}
{"operation":"apt_remove","packages":["curl"]}
{"operation":"apt_update"}
{"operation":"apt_upgrade"}
```

Service actions are exactly `start`, `stop`, `restart`, and `enable`.
Unknown fields, versions, operations, signals, caller identities, and
`approved` booleans are rejected. Unit identifiers are literal non-template
`.service` names, at most 255 bytes; no paths, escapes, globs or options.
Package identifiers are at most 128 bytes, 64 unique names per request; no
user-supplied versions, architectures, repositories or APT options.

The `Prepare` result is:

```text
{
  "plan": {
    "version": 1,
    "id": "<256-bit random nonce, lowercase hex>",
    "caller": {
      "unique_name": ":1.123",
      "uid": 1000,
      "pid": 1234,
      "process_start": 98765,
      "session": "/org/freedesktop/login1/session/_31"
    },
    "action_id": "org.sentia.system.service-restart",
    "operation": { "operation": "service", "action": "restart", "unit": "ssh.service" },
    "argument_digest": "<SHA-256>",
    "state": { "<operation-specific current state>": "..." },
    "issued_at": 1700000000,
    "expires_at": 1700000120
  },
  "digest": "<SHA-256 of the broker's serialized entire plan>"
}
```

Times are Unix seconds. Return the **original** `id` and outer `digest`; do not
recompute either from a reformatted preview. The broker retains the canonical
plan in memory; it does not accept a client-edited plan back. The nested native
APT plan uses its separately versioned worker protocol, not the router's
`sentia.v1` request envelope.

The trusted desktop frontend must hold **one D-Bus connection** across preview
and apply. It must display the exact operation, state, package deltas and
unknown effects as plain text, obtain an explicit user confirmation, then
invoke `Apply`. Merely receiving a model/tool proposal must never trigger
`Apply`. The model/tool registry must not register this method. Confirmation
is a frontend concern, not a credential; a client boolean never grants root.
A desktop polkit authentication agent must already be running.

The broker obtains UID/PID from the bus daemon and the user's active, local,
class=`user` session from logind. Both are checked again after authentication.
It uses polkit's `system-bus-name` subject with the unique connection name,
empty caller-controlled details, and `AllowUserInteraction`. The nine action
messages are fixed in `config/polkit/org.sentia.system.policy`. All actions
require non-cached `auth_admin` for active sessions; inactive/remote sessions
are rejected independently by the service and denial-only polkit rule.

Plans expire in 120 seconds and are single-attempt, including authentication
cancellation, denial and failed execution. There are at most eight pending
plans per connection and 256 in total. Restarting the broker invalidates all
plans. The operation gate serializes broker work. Changed state, caller,
session, expiry or digest requires a fresh preview.

Successful service execution returns JSON with `version`, `status`, and
post-operation `state`. Process execution returns `terminated` or
`signal_sent_process_still_running`, with `signal: "SIGTERM"`; it never
escalates a requested process kill to SIGKILL. APT returns the native worker
response. Errors are D-Bus errors with fixed machine-readable identifiers,
for example `authorization_denied`, `state_changed_prepare_again`,
`caller_changed`, `plan_expired`, `unknown_or_used_plan`, or `broker_busy`.
Subprocess stderr and full package/prompt text are not written to broker logs.
Audit logs record only operation action, numeric caller UID and outcome.

## Execution boundary

* Service mutation uses only `/usr/bin/systemctl`, fixed switches and argv.
  Loaded-state, activation-state, unit configuration and drop-in hashes are
  bound in the preview. Transient/stale units and non-root-owned/writable
  configuration chains are rejected. Unit files are opened without following
  a final symlink after checking their canonical root-owned paths.
* Process termination opens a **pidfd** before its final state check and
  signals that descriptor. PID 0, PID 1, negative/overflow identifiers and
  the broker itself cannot be targeted. Start ticks, UID and process name
  are bound. Only SIGTERM is available, with a bounded three-second wait.
* Package resolution/execution is delegated through pipes to the separate
  `/usr/libexec/sentia/sentia-apt-worker` executable. The Rust broker does
  **not** link libapt-pkg.
* Executables and their ancestor paths must be root-owned, non-symlink and
  not group/world writable. The environment is cleared and rebuilt with a
  fixed PATH, C locale and noninteractive debconf. No shell, `eval`, arbitrary
  command, environment, file deletion, account, firewall or network API exists.
* Subprocess stdout/stderr are capped at 1 MiB each. Service/plan execution
  times out after 30 seconds; package apply after 900 seconds. Only the
  broker's own helper process group is terminated on timeout/output overflow.
  APT interruption may require an administrator-led dpkg recovery; no
  unapproved automatic repair command is executed.

The systemd sandbox protects homes, provider credential paths, kernel state
and devices, limits capabilities/resources and prohibits new privileges.
APT must write dpkg-managed `/usr`, `/etc` and `/var`; these are explicitly
writable, not misleadingly advertised as immutable. Maintainer scripts inherit
the same restrictions; compatibility with particular packages requires VM
testing. This is not a root-adversary boundary: another administrator can
modify systemd configuration or package state concurrently. The worker must
perform package plan comparison under the native package-manager locks.

## Native worker contract

See `native/sentia-apt/protocol/broker-contract.json`. The fixed helper receives
one bounded JSON document on stdin and responds with JSON on stdout.

Plan request:

```json
{"protocol_version":"1.0","request_id":"sentia-root-plan","operation":"apt_install","arguments":{"mode":"plan","packages":["curl"]}}
```

Only `result.canonical_plan` and `result.plan_digest` are bound into broker
state; response timestamps and diagnostics do not alter approval. After
successful polkit authorization, the broker creates an execute request with
`arguments.mode="execute"` and an `approval` object containing the exact
native digest, hashed broker caller/session identity, consumed plan nonce as
`authorization_id`, and UTC expiry. `allow_source_change`,
`allow_essential_removal`, and `allow_held_change` are always false.
These are root-to-worker assertions, **not** a transferable authentication
token accepted from clients. The worker independently requires root and
rejects changed plans under its lock. Lock acquisition mode must not itself
be included in the native plan digest.

## Validation

From the integrated repository root inside the designated **Debian 13/Trixie
builder**, after its root workspace lock has been resolved for the target
toolchain (Rust 1.85 or later):

```sh
flock /home/ubuntu/sentia-linux/artifacts/.locks/heavy.lock \
  cargo test --manifest-path crates/sentia-privileged/Cargo.toml --locked -j 2
```

`tests/privileged/boundary.rs` covers schema/argument rejection, nonce/digest
integrity, expiry/replay, all caller-binding components, changed state and
process start ticks, capacity limits, policy invariants and native worker IPC.
`tests/privileged/isolated_bus.rs` starts a private abstract-socket D-Bus daemon
and the real Rust service, using controlled logind/polkit peers. It checks
unique-name/UID/PID binding, denial, replay, remote/inactive rejection,
session loss during authorization, and SIGTERM of its own unprivileged child.
It never connects to the host system bus or mutates host privileged state.

The isolated peer is **not real polkit authentication**. Real desktop password
prompts, installed service startup, actual service/APT mutations and the
systemd sandbox's package compatibility must be exercised in a disposable VM.
Do not treat fixture authorization responses as release evidence.

### Recorded implementation check (2026-09-12)

On the development host with Rust/Cargo 1.75, an earlier standalone test lock
compiled successfully: **14 boundary tests, one bounded-output unit test and
one isolated-bus integration test passed**. XML policy parsing passed.
`systemd-analyze verify` reported the expected not-yet-installed executable,
not a unit syntax error. `cargo fmt --check` was unavailable because the
designated builder had not installed rustfmt.

Those are **host-only implementation checks, not Debian target build or
qualification evidence**. The Ubuntu-only standalone lock was subsequently
removed, not carried into the product dependency resolution. The crate now
declares the Trixie Rust 1.85 baseline; no target dependency should be downgraded
merely to accommodate Ubuntu's Rust 1.75. The root workspace owner must resolve
and commit the product lockfile in the target builder. Target compilation and
the exact ready builder invocation are pending the designated builder owner.

Not yet verified: actual polkit authentication/desktop prompts, installed
systemd sandbox startup, real service/APT changes, Debian 13 target compilation
and Debian 13 VM operation.
An unprivileged attempt to launch the real polkit daemon on a private bus
exited immediately; no host polkit daemon, policy or privileged state was
modified. Native APT integration additionally requires its independent worker
to exclude the differing plan/execute `with_lock` flag from its canonical
digest. The worker owner's updated source now places that flag only in
diagnostics and re-resolves a second time with the native cache locked before
comparing the execution digest. Its current `broker-contract.json` agrees with
the broker adapter above. This source-level contract check is not an executed
root package transaction or a GUI/polkit acceptance result.
