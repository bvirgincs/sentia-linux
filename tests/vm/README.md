# QEMU VM acceptance harness

`vm_harness.py` creates disposable qcow2 disks, copies an OVMF variable store,
uses a private QMP Unix socket, captures serial output, and provides only QEMU
user-mode networking. It never exposes a host block device or shared directory.

An install run requires a reviewed scenario that drives the real Calamares GUI
with QMP keyboard/pointer events and records screenshots. The checked-in
scenario is deliberately only a template and cannot produce a passing install:
real image-specific coordinates and serial assertions must replace it.

```sh
python3 tests/vm/vm_harness.py install \
  --image-kind production \
  --iso artifacts/sentia.iso \
  --scenario tests/vm/scenarios/calamares-reviewed.json \
  --success-serial-regex 'calamares.*finished'

python3 tests/vm/vm_harness.py disk-boot \
  --image-kind production \
  --disk artifacts/vm/<install-run>/sentia.qcow2 \
  --success-serial-regex 'sentia-installed-acceptance-ready'
```

Test images may expose a prebuilt, ephemeral authorized key hook. The harness
does not inject credentials into an ISO. Supply the corresponding private key
from outside Git with `--test-ssh-key` and `--ssh-user`; SSH is forwarded only
to loopback and a fixed read-only guest probe is run. `--image-kind production`
rejects all test authentication options.

Each invocation writes `evidence.json` with input hashes, environment, guest
configuration, log references, status, and failure reason. A dry run is always
recorded as `not-run`, never as a pass. Until real ISO inputs exist, no install
or disk-boot acceptance claim is made.
