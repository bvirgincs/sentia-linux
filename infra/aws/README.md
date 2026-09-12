# Disposable AWS VM runner

`runner.py` owns at most one tagged EC2 runner in `us-east-1`. It selects the
cheaper currently offered approved type, verifies the official Canonical
Ubuntu 24.04 AMI, uses encrypted gp3 storage and IMDSv2, and enables EC2 nested
virtualization. Existing narrow SSM access is preferred. The fallback creates
one key outside Git and allows SSH only from the orchestrator's current `/32`.

The private state directory contains exact resource IDs, timestamps, access
data, and the cost ledger. Keep it outside the checkout with mode `0700`:

```sh
export SENTIA_AWS_STATE_DIR="$HOME/.local/state/sentia/aws-runner"
python3 infra/aws/runner.py preflight
python3 infra/aws/runner.py provision --hours 12 --volume-gib 120
python3 infra/aws/runner.py wait-ready
python3 infra/aws/runner.py prove-nested-kvm
python3 infra/aws/runner.py status
```

Use `python3 infra/aws/runner.py shell` for SSM (or restricted SSH fallback)
without printing or copying secret material. Start a second host-side guard
with `python3 infra/aws/runner.py guard`; the instance also installs its own
expiry timer before package installation, and OS shutdown terminates EC2.

Reconcile an interrupted operation without deleting unrecorded resources:

```sh
python3 infra/aws/runner.py reconcile
```

Cleanup uses only ledger-recorded resources and verifies ownership tags:

```sh
python3 infra/aws/runner.py cleanup
```

Stopping the instance is not cleanup. The cleanup command terminates compute,
waits for termination, confirms/deletes recorded volumes, removes an ephemeral
key if one exists, and deletes the dedicated security group.

