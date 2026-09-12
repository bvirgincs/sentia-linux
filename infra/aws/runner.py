#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Provision one bounded, disposable Sentia nested-KVM runner in AWS."""

from __future__ import annotations

import argparse
import contextlib
import datetime as dt
import fcntl
import ipaddress
import json
import os
import re
import secrets
import socket
import stat
import subprocess
import sys
import time
import urllib.request
from decimal import Decimal
from pathlib import Path
from typing import Any, Iterator, Sequence

REGION = "us-east-1"
AMI_PARAMETER = (
    "/aws/service/canonical/ubuntu/server/24.04/stable/current/"
    "amd64/hvm/ebs-gp3/ami-id"
)
CANONICAL_OWNER = "099720109477"
APPROVED_TYPES = ("m7i.2xlarge", "m8i.2xlarge")
PROJECT_TAGS = {
    "SentiaProject": "Sentia",
    "SentiaPurpose": "vm-test-runner",
    "ManagedBy": "sentia-aws-runner",
}
DEFAULT_STATE = Path.home() / ".local/state/sentia/aws-runner"
DEFAULT_RUNTIME_HOURS = Decimal("12")
DEFAULT_VOLUME_GIB = 120
DEFAULT_TRANSFER_GIB = Decimal("20")
TRANSFER_USD_PER_GIB = Decimal("0.09")
BUDGET_USD = Decimal("25")
SAFETY_RESERVE_USD = Decimal("5")
COST_MULTIPLIER = Decimal("1.25")
GUARD_THRESHOLD_USD = Decimal("22")
UTC = dt.timezone.utc


class RunnerError(RuntimeError):
    pass


def now() -> dt.datetime:
    return dt.datetime.now(UTC)


def iso_time(value: dt.datetime) -> str:
    return value.astimezone(UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def error_code(stderr: str) -> str:
    match = re.search(r"An error occurred \(([^)]+)\)", stderr)
    return match.group(1) if match else "AwsCliError"


def aws(
    args: Sequence[str],
    *,
    json_output: bool = True,
    check: bool = True,
) -> Any:
    command = ["aws", *args]
    if json_output:
        command += ["--output", "json"]
    proc = subprocess.run(command, text=True, capture_output=True)
    if proc.returncode and check:
        operation = " ".join(args[:2])
        raise RunnerError(f"{operation} failed ({error_code(proc.stderr)})")
    if not check:
        return proc
    if not json_output:
        return proc.stdout
    return json.loads(proc.stdout or "{}")


def state_dir() -> Path:
    path = Path(os.environ.get("SENTIA_AWS_STATE_DIR", DEFAULT_STATE)).expanduser().resolve()
    repo = Path(__file__).resolve().parents[2]
    if path == repo or repo in path.parents:
        raise RunnerError("SENTIA_AWS_STATE_DIR must be outside the Git worktree")
    path.mkdir(parents=True, exist_ok=True, mode=0o700)
    os.chmod(path, 0o700)
    return path


def read_json(name: str) -> dict[str, Any]:
    path = state_dir() / name
    if not path.is_file():
        raise RunnerError(f"missing private state file: {name}; run preflight/provision first")
    return json.loads(path.read_text())


def write_json(name: str, data: dict[str, Any]) -> None:
    path = state_dir() / name
    temporary = path.with_suffix(path.suffix + ".new")
    temporary.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n")
    os.chmod(temporary, 0o600)
    temporary.replace(path)


@contextlib.contextmanager
def operation_lock() -> Iterator[None]:
    lock_path = state_dir() / "operation.lock"
    with lock_path.open("a+") as lock:
        os.chmod(lock_path, 0o600)
        fcntl.flock(lock, fcntl.LOCK_EX)
        yield


def tags(run_id: str, expires_at: str) -> dict[str, str]:
    return {
        **PROJECT_TAGS,
        "SentiaRunId": run_id,
        "ExpiresAt": expires_at,
        "AuthorizedBudgetUSD": str(BUDGET_USD),
    }


def tag_spec(resource_type: str, values: dict[str, str]) -> str:
    entries = ",".join(
        f"{{Key={key},Value={value}}}" for key, value in values.items()
    )
    return f"ResourceType={resource_type},Tags=[{entries}]"


def pricing(filters: Sequence[tuple[str, str]]) -> list[tuple[str, Decimal, str]]:
    args = [
        "pricing",
        "get-products",
        "--region",
        REGION,
        "--service-code",
        "AmazonEC2",
        "--max-results",
        "100",
        "--filters",
        *[
            f"Type=TERM_MATCH,Field={field},Value={value}"
            for field, value in filters
        ],
    ]
    result: list[tuple[str, Decimal, str]] = []
    for raw in aws(args).get("PriceList", []):
        product = json.loads(raw)
        for term in product.get("terms", {}).get("OnDemand", {}).values():
            for dimension in term.get("priceDimensions", {}).values():
                value = dimension.get("pricePerUnit", {}).get("USD")
                if value is not None:
                    result.append(
                        (
                            dimension.get("unit", ""),
                            Decimal(value),
                            dimension.get("description", ""),
                        )
                    )
    return result


def dry_run(args: Sequence[str], *, allow_not_found: bool = False) -> bool:
    proc = aws(args, check=False)
    code = error_code(proc.stderr)
    if code == "DryRunOperation":
        return True
    if allow_not_found and code in {
        "InvalidInstanceID.NotFound",
        "InvalidGroup.NotFound",
        "InvalidKeyPair.NotFound",
    }:
        return True
    return False


def permissions_allowed(actions: Sequence[str]) -> bool:
    identity = aws(["sts", "get-caller-identity", "--region", REGION])
    try:
        result = aws(
            [
                "iam",
                "simulate-principal-policy",
                "--policy-source-arn",
                identity["Arn"],
                "--action-names",
                *actions,
            ]
        )
    except RunnerError:
        return False
    decisions = {
        row["EvalActionName"]: row["EvalDecision"]
        for row in result.get("EvaluationResults", [])
    }
    return all(decisions.get(action) == "allowed" for action in actions)


def public_subnets(vpc_id: str) -> list[dict[str, Any]]:
    subnets = aws(
        [
            "ec2",
            "describe-subnets",
            "--region",
            REGION,
            "--filters",
            f"Name=vpc-id,Values={vpc_id}",
            "Name=state,Values=available",
        ]
    ).get("Subnets", [])
    route_tables = aws(
        [
            "ec2",
            "describe-route-tables",
            "--region",
            REGION,
            "--filters",
            f"Name=vpc-id,Values={vpc_id}",
        ]
    ).get("RouteTables", [])

    def has_internet_gateway(subnet: dict[str, Any]) -> bool:
        specific = [
            table
            for table in route_tables
            if any(
                association.get("SubnetId") == subnet["SubnetId"]
                for association in table.get("Associations", [])
            )
        ]
        selected = specific or [
            table
            for table in route_tables
            if any(
                association.get("Main")
                for association in table.get("Associations", [])
            )
        ]
        return any(
            route.get("DestinationCidrBlock") == "0.0.0.0/0"
            and route.get("GatewayId", "").startswith("igw-")
            and route.get("State") == "active"
            for table in selected
            for route in table.get("Routes", [])
        )

    return [
        subnet
        for subnet in subnets
        if subnet.get("MapPublicIpOnLaunch") and has_internet_gateway(subnet)
    ]


def narrow_ssm_profile() -> tuple[str | None, str]:
    try:
        profiles = aws(["iam", "list-instance-profiles"]).get("InstanceProfiles", [])
        allowed = {"AmazonSSMManagedInstanceCore", "CloudWatchAgentServerPolicy"}
        for profile in profiles:
            roles = profile.get("Roles", [])
            if len(roles) != 1:
                continue
            role_name = roles[0]["RoleName"]
            attached = aws(
                ["iam", "list-attached-role-policies", "--role-name", role_name]
            ).get("AttachedPolicies", [])
            inline = aws(["iam", "list-role-policies", "--role-name", role_name]).get(
                "PolicyNames", []
            )
            policy_names = {
                policy["PolicyArn"].rsplit("/", 1)[-1] for policy in attached
            }
            if (
                "AmazonSSMManagedInstanceCore" in policy_names
                and not inline
                and policy_names <= allowed
            ):
                return profile["InstanceProfileName"], "narrow-ssm-candidate"
        return None, "no-narrow-candidate"
    except RunnerError:
        return None, "iam-inspection-denied"


def run_arguments(
    data: dict[str, Any],
    security_group_id: str,
    *,
    dry: bool,
    user_data_path: Path | None = None,
) -> list[str]:
    values = tags(data["run_id"], data["expires_at"])
    args = [
        "ec2",
        "run-instances",
        "--region",
        REGION,
        "--image-id",
        data["ami_id"],
        "--instance-type",
        data["instance_type"],
        "--count",
        "1",
        "--subnet-id",
        data["subnet_id"],
        "--security-group-ids",
        security_group_id,
        "--associate-public-ip-address",
        "--cpu-options",
        "NestedVirtualization=enabled",
        "--metadata-options",
        "HttpTokens=required,HttpEndpoint=enabled,HttpPutResponseHopLimit=1,InstanceMetadataTags=disabled",
        "--instance-initiated-shutdown-behavior",
        "terminate",
        "--block-device-mappings",
        json.dumps(
            [
                {
                    "DeviceName": data["root_device_name"],
                    "Ebs": {
                        "DeleteOnTermination": True,
                        "Encrypted": True,
                        "Iops": 3000,
                        "Throughput": 125,
                        "VolumeSize": data["volume_gib"],
                        "VolumeType": "gp3",
                    },
                }
            ],
            separators=(",", ":"),
        ),
        "--tag-specifications",
        tag_spec("instance", values),
        tag_spec("volume", values),
    ]
    if data.get("ssm_profile"):
        args += ["--iam-instance-profile", f"Name={data['ssm_profile']}"]
    if data.get("key_name"):
        args += ["--key-name", data["key_name"]]
    if user_data_path:
        args += ["--user-data", f"file://{user_data_path}"]
    if dry:
        args.append("--dry-run")
    return args


def command_preflight(_: argparse.Namespace) -> None:
    with operation_lock():
        aws(["sts", "get-caller-identity", "--region", REGION])
        ami_id = aws(
            [
                "ssm",
                "get-parameter",
                "--region",
                REGION,
                "--name",
                AMI_PARAMETER,
            ]
        )["Parameter"]["Value"]
        image = aws(
            ["ec2", "describe-images", "--region", REGION, "--image-ids", ami_id]
        )["Images"][0]
        if (
            image.get("OwnerId") != CANONICAL_OWNER
            or not image.get("Name", "").startswith(
                "ubuntu/images/hvm-ssd-gp3/ubuntu-noble-24.04-amd64-server-"
            )
        ):
            raise RunnerError("Canonical Ubuntu 24.04 AMI verification failed")

        vpcs = aws(
            [
                "ec2",
                "describe-vpcs",
                "--region",
                REGION,
                "--filters",
                "Name=is-default,Values=true",
            ]
        ).get("Vpcs", [])
        if len(vpcs) != 1:
            raise RunnerError("exactly one default VPC is required")
        vpc_id = vpcs[0]["VpcId"]
        subnets = public_subnets(vpc_id)
        if not subnets:
            raise RunnerError("no eligible public default-VPC subnet was found")

        prices: dict[str, Decimal] = {}
        offerings: dict[str, list[str]] = {}
        location = "US East (N. Virginia)"
        for instance_type in APPROVED_TYPES:
            values = pricing(
                [
                    ("instanceType", instance_type),
                    ("location", location),
                    ("operatingSystem", "Linux"),
                    ("tenancy", "Shared"),
                    ("preInstalledSw", "NA"),
                    ("capacitystatus", "Used"),
                ]
            )
            hourly = [price for unit, price, _ in values if unit == "Hrs" and price > 0]
            if not hourly:
                raise RunnerError(f"no on-demand price resolved for {instance_type}")
            prices[instance_type] = min(hourly)
            rows = aws(
                [
                    "ec2",
                    "describe-instance-type-offerings",
                    "--region",
                    REGION,
                    "--location-type",
                    "availability-zone",
                    "--filters",
                    f"Name=instance-type,Values={instance_type}",
                ]
            ).get("InstanceTypeOfferings", [])
            offerings[instance_type] = sorted(row["Location"] for row in rows)

        eligible = sorted(
            [
                (
                prices[instance_type],
                instance_type,
                subnet,
                )
                for instance_type, zones in offerings.items()
                for subnet in subnets
                if subnet["AvailabilityZone"] in zones
            ],
            key=lambda item: (
                item[0],
                item[1],
                item[2]["AvailabilityZone"],
                item[2]["SubnetId"],
            ),
        )
        if not eligible:
            raise RunnerError("approved instance types are unavailable in eligible subnets")
        compute_price, instance_type, subnet = eligible[0]

        ebs_values = pricing([("location", location), ("volumeApiName", "gp3")])
        ebs_prices = [
            price
            for unit, price, description in ebs_values
            if unit == "GB-Mo" and price > 0 and "storage" in description.lower()
        ]
        if not ebs_prices:
            raise RunnerError("no gp3 storage price resolved")
        ebs_price = min(ebs_prices)
        ipv4_values = pricing([("location", location), ("productFamily", "IP Address")])
        ipv4_prices = [
            price
            for unit, price, description in ipv4_values
            if unit == "Hrs"
            and price > 0
            and ("public" in description.lower() or "ipv4" in description.lower())
        ]
        ipv4_price = min(ipv4_prices) if ipv4_prices else Decimal("0.005")

        with urllib.request.urlopen("https://checkip.amazonaws.com", timeout=10) as reply:
            source_ip = reply.read().decode().strip()
        ipaddress.IPv4Address(source_ip)

        default_groups = aws(
            [
                "ec2",
                "describe-security-groups",
                "--region",
                REGION,
                "--filters",
                f"Name=vpc-id,Values={vpc_id}",
                "Name=group-name,Values=default",
            ]
        ).get("SecurityGroups", [])
        if len(default_groups) != 1:
            raise RunnerError("default security group could not be resolved for dry-run")

        profile, profile_status = narrow_ssm_profile()
        run_id = secrets.token_hex(8)
        probe = {
            "run_id": run_id,
            "expires_at": iso_time(now() + dt.timedelta(hours=1)),
            "ami_id": ami_id,
            "instance_type": instance_type,
            "subnet_id": subnet["SubnetId"],
            "root_device_name": image["RootDeviceName"],
            "volume_gib": DEFAULT_VOLUME_GIB,
            "ssm_profile": profile,
        }
        if profile and not dry_run(
            run_arguments(probe, default_groups[0]["GroupId"], dry=True)
        ):
            profile = None
            profile_status = "candidate-not-passable"
            probe["ssm_profile"] = None
        if not dry_run(run_arguments(probe, default_groups[0]["GroupId"], dry=True)):
            raise RunnerError("RunInstances dry-run was not authorized")
        if not dry_run(
            [
                "ec2",
                "create-security-group",
                "--region",
                REGION,
                "--group-name",
                f"sentia-dryrun-{run_id}",
                "--description",
                "Sentia permission preflight",
                "--vpc-id",
                vpc_id,
                "--dry-run",
            ]
        ):
            raise RunnerError("CreateSecurityGroup dry-run was not authorized")
        access = "ssm" if profile else "ssh"
        teardown_actions = [
            "ec2:TerminateInstances",
            "ec2:DeleteSecurityGroup",
            "ec2:DeleteVolume",
        ]
        if access == "ssh":
            teardown_actions.append("ec2:DeleteKeyPair")
            if not dry_run(
                [
                    "ec2",
                    "create-key-pair",
                    "--region",
                    REGION,
                    "--key-name",
                    f"sentia-dryrun-{run_id}",
                    "--dry-run",
                ]
            ):
                raise RunnerError("CreateKeyPair dry-run was not authorized")
            if not dry_run(
                [
                    "ec2",
                    "authorize-security-group-ingress",
                    "--region",
                    REGION,
                    "--group-id",
                    default_groups[0]["GroupId"],
                    "--protocol",
                    "tcp",
                    "--port",
                    "22",
                    "--cidr",
                    f"{source_ip}/32",
                    "--dry-run",
                ]
            ):
                raise RunnerError("restricted SSH ingress dry-run was not authorized")
        if not permissions_allowed(teardown_actions):
            raise RunnerError("IAM simulation did not allow all required teardown actions")

        active = aws(
            [
                "ec2",
                "describe-instances",
                "--region",
                REGION,
                "--filters",
                "Name=tag:SentiaProject,Values=Sentia",
                "Name=tag:SentiaPurpose,Values=vm-test-runner",
                "Name=instance-state-name,Values=pending,running,stopping,stopped",
            ]
        ).get("Reservations", [])
        active_count = sum(len(item.get("Instances", [])) for item in active)
        if active_count:
            raise RunnerError(
                "an existing Sentia test runner is active; reconcile private state first"
            )

        data = {
            "schema_version": 1,
            "generated_at": iso_time(now()),
            "region": REGION,
            "ami_id": ami_id,
            "ami_name": image["Name"],
            "ami_owner_verified": True,
            "root_device_name": image["RootDeviceName"],
            "vpc_id": vpc_id,
            "subnet_id": subnet["SubnetId"],
            "availability_zone": subnet["AvailabilityZone"],
            "instance_type": instance_type,
            "compute_hourly_usd": str(compute_price),
            "gp3_gb_month_usd": str(ebs_price),
            "public_ipv4_hourly_usd": str(ipv4_price),
            "source_cidr": f"{source_ip}/32",
            "ssm_profile": profile,
            "ssm_profile_check": profile_status,
            "access_method": access,
            "eligible_public_subnet_count": len(subnets),
            "offering_az_counts": {
                key: len(value) for key, value in offerings.items()
            },
        }
        write_json("preflight.json", data)
        print("Preflight passed: authentication, AMI ownership, network, pricing,")
        print("nested-virtualization offerings, launch, and teardown permissions verified.")
        print(
            f"Selected {instance_type} at ${compute_price:.4f}/hour; "
            f"gp3 ${ebs_price:.3f}/GB-month; IPv4 ${ipv4_price:.3f}/hour."
        )
        print(f"Access method: {access}; private identifiers remain in local state.")


def budget(preflight: dict[str, Any], runtime_hours: Decimal, volume_gib: int) -> dict[str, Any]:
    compute = Decimal(preflight["compute_hourly_usd"]) * runtime_hours
    ebs = (
        Decimal(preflight["gp3_gb_month_usd"])
        * Decimal(volume_gib)
        * runtime_hours
        / Decimal(730)
    )
    ipv4 = Decimal(preflight["public_ipv4_hourly_usd"]) * runtime_hours
    transfer = DEFAULT_TRANSFER_GIB * TRANSFER_USD_PER_GIB
    metered = compute + ebs + ipv4 + transfer
    projected = metered * COST_MULTIPLIER + SAFETY_RESERVE_USD
    if projected >= BUDGET_USD:
        raise RunnerError(
            f"conservative projected cap ${projected:.2f} does not fit ${BUDGET_USD}"
        )
    return {
        "schema_version": 1,
        "authorized_total_usd": str(BUDGET_USD),
        "guard_threshold_usd": str(GUARD_THRESHOLD_USD),
        "maximum_runtime_hours": str(runtime_hours),
        "volume_gib": volume_gib,
        "maximum_transfer_gib": str(DEFAULT_TRANSFER_GIB),
        "transfer_usd_per_gib": str(TRANSFER_USD_PER_GIB),
        "compute_reserved_usd": str(compute.quantize(Decimal("0.0001"))),
        "storage_reserved_usd": str(ebs.quantize(Decimal("0.0001"))),
        "ipv4_reserved_usd": str(ipv4.quantize(Decimal("0.0001"))),
        "transfer_reserved_usd": str(transfer.quantize(Decimal("0.0001"))),
        "cost_multiplier": str(COST_MULTIPLIER),
        "safety_and_cleanup_reserve_usd": str(SAFETY_RESERVE_USD),
        "conservative_projected_cap_usd": str(projected.quantize(Decimal("0.01"))),
        "created_at": iso_time(now()),
    }


def append_resource(ledger: dict[str, Any], kind: str, identifier: str) -> None:
    ledger["resources"].append(
        {"kind": kind, "id": identifier, "created_at": iso_time(now())}
    )
    write_json("ledger.json", ledger)


def user_data(expires_at: str, runtime_seconds: int) -> str:
    return f"""#!/bin/bash
set -Eeuo pipefail
install -d -m 0755 /var/lib/sentia-runner /opt/sentia
cat >/etc/systemd/system/sentia-runner-expiry.service <<'EOF'
[Unit]
Description=Terminate expired disposable Sentia runner
[Service]
Type=oneshot
ExecStart=/sbin/shutdown -h now
EOF
cat >/etc/systemd/system/sentia-runner-expiry.timer <<'EOF'
[Unit]
Description=Bound disposable Sentia runner lifetime
[Timer]
OnActiveSec={runtime_seconds}s
AccuracySec=30s
Unit=sentia-runner-expiry.service
[Install]
WantedBy=timers.target
EOF
systemctl daemon-reload
systemctl enable --now sentia-runner-expiry.timer
printf '%s\\n' '{expires_at}' >/var/lib/sentia-runner/expires-at
export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y --no-install-recommends \
  qemu-system-x86 qemu-utils ovmf cloud-image-utils xorriso \
  jq socat curl openssh-client ca-certificates cpu-checker
usermod -aG kvm ubuntu
chown ubuntu:ubuntu /opt/sentia
test -c /dev/kvm
printf '%s\\n' ready >/var/lib/sentia-runner/ready
"""


def command_provision(args: argparse.Namespace) -> None:
    with operation_lock():
        preflight = read_json("preflight.json")
        generated = dt.datetime.fromisoformat(
            preflight["generated_at"].replace("Z", "+00:00")
        )
        if now() - generated > dt.timedelta(hours=2):
            raise RunnerError("preflight is older than two hours; rerun it")
        runtime_hours = Decimal(str(args.hours))
        if runtime_hours <= 0 or runtime_hours > Decimal("24"):
            raise RunnerError("runner lifetime must be greater than 0 and at most 24 hours")
        if args.volume_gib < 64 or args.volume_gib > 200:
            raise RunnerError("gp3 volume size must be between 64 and 200 GiB")

        active = aws(
            [
                "ec2",
                "describe-instances",
                "--region",
                REGION,
                "--filters",
                "Name=tag:SentiaProject,Values=Sentia",
                "Name=tag:SentiaPurpose,Values=vm-test-runner",
                "Name=instance-state-name,Values=pending,running,stopping,stopped",
            ]
        ).get("Reservations", [])
        if any(item.get("Instances") for item in active):
            raise RunnerError("refusing to create a second Sentia test runner")

        cost = budget(preflight, runtime_hours, args.volume_gib)
        write_json("budget.json", cost)
        start = now()
        expires = start + dt.timedelta(hours=float(runtime_hours))
        run_id = secrets.token_hex(8)
        ledger: dict[str, Any] = {
            "schema_version": 1,
            "run_id": run_id,
            "status": "guard-established",
            "created_at": iso_time(start),
            "expires_at": iso_time(expires),
            "region": REGION,
            "instance_type": preflight["instance_type"],
            "access_method": preflight["access_method"],
            "resources": [],
        }
        write_json("ledger.json", ledger)

        values = tags(run_id, ledger["expires_at"])
        group_name = f"sentia-vm-runner-{run_id}"
        try:
            group = aws(
                [
                    "ec2",
                    "create-security-group",
                    "--region",
                    REGION,
                    "--group-name",
                    group_name,
                    "--description",
                    "Restricted disposable Sentia VM test runner",
                    "--vpc-id",
                    preflight["vpc_id"],
                    "--tag-specifications",
                    tag_spec("security-group", values),
                ]
            )
            group_id = group["GroupId"]
            append_resource(ledger, "security-group", group_id)

            key_name = None
            if preflight["access_method"] == "ssh":
                key_name = f"sentia-vm-runner-{run_id}"
                key = aws(
                    [
                        "ec2",
                        "create-key-pair",
                        "--region",
                        REGION,
                        "--key-name",
                        key_name,
                        "--key-type",
                        "ed25519",
                        "--tag-specifications",
                        tag_spec("key-pair", values),
                    ]
                )
                key_path = state_dir() / "runner-ssh.key"
                key_path.write_text(key["KeyMaterial"])
                os.chmod(key_path, stat.S_IRUSR | stat.S_IWUSR)
                append_resource(ledger, "key-pair", key_name)
                aws(
                    [
                        "ec2",
                        "authorize-security-group-ingress",
                        "--region",
                        REGION,
                        "--group-id",
                        group_id,
                        "--protocol",
                        "tcp",
                        "--port",
                        "22",
                        "--cidr",
                        preflight["source_cidr"],
                    ]
                )

            private_run = {
                **preflight,
                "run_id": run_id,
                "expires_at": ledger["expires_at"],
                "volume_gib": args.volume_gib,
                "key_name": key_name,
            }
            data_path = state_dir() / "user-data.sh"
            data_path.write_text(
                user_data(ledger["expires_at"], int(runtime_hours * Decimal(3600)))
            )
            os.chmod(data_path, 0o600)
            response = aws(
                run_arguments(
                    private_run, group_id, dry=False, user_data_path=data_path
                )
            )
            instance = response["Instances"][0]
            instance_id = instance["InstanceId"]
            append_resource(ledger, "instance", instance_id)
            ledger["status"] = "provisioning"
            write_json("ledger.json", ledger)

            if not dry_run(
                [
                    "ec2",
                    "terminate-instances",
                    "--region",
                    REGION,
                    "--instance-ids",
                    instance_id,
                    "--dry-run",
                ]
            ):
                raise RunnerError("post-launch termination permission check failed")

            aws(
                [
                    "ec2",
                    "wait",
                    "instance-running",
                    "--region",
                    REGION,
                    "--instance-ids",
                    instance_id,
                ],
                json_output=False,
            )
            aws(
                [
                    "ec2",
                    "wait",
                    "instance-status-ok",
                    "--region",
                    REGION,
                    "--instance-ids",
                    instance_id,
                ],
                json_output=False,
            )
            details = aws(
                [
                    "ec2",
                    "describe-instances",
                    "--region",
                    REGION,
                    "--instance-ids",
                    instance_id,
                ]
            )["Reservations"][0]["Instances"][0]
            ledger["started_at"] = details["LaunchTime"]
            ledger["public_ipv4"] = details.get("PublicIpAddress")
            for mapping in details.get("BlockDeviceMappings", []):
                volume_id = mapping.get("Ebs", {}).get("VolumeId")
                if volume_id:
                    append_resource(ledger, "volume", volume_id)
            ledger["status"] = "running"
            write_json("ledger.json", ledger)
        except Exception:
            ledger["status"] = "provision-failed"
            write_json("ledger.json", ledger)
            cleanup(ledger, quiet=True)
            raise

        print("Runner launched with a pre-established budget guard and hard expiry.")
        print(
            f"Configuration: {preflight['instance_type']}, 8 vCPU, 32 GiB RAM, "
            f"{args.volume_gib} GiB encrypted gp3."
        )
        print(
            f"Maximum runtime: {runtime_hours} hours; conservative reserved cap: "
            f"${cost['conservative_projected_cap_usd']} of ${BUDGET_USD}."
        )
        print("Exact identifiers and timestamps are stored only in private local state.")


def resource(ledger: dict[str, Any], kind: str) -> str | None:
    for item in ledger.get("resources", []):
        if item["kind"] == kind:
            return item["id"]
    return None


def expected_tags(instance: dict[str, Any], ledger: dict[str, Any]) -> bool:
    actual = {tag["Key"]: tag["Value"] for tag in instance.get("Tags", [])}
    return all(actual.get(key) == value for key, value in PROJECT_TAGS.items()) and (
        actual.get("SentiaRunId") == ledger["run_id"]
    )


def wait_for_ssm(instance_id: str, timeout: int = 900) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = aws(
            [
                "ssm",
                "describe-instance-information",
                "--region",
                REGION,
                "--filters",
                f"Key=InstanceIds,Values={instance_id}",
            ]
        )
        entries = result.get("InstanceInformationList", [])
        if entries and entries[0].get("PingStatus") == "Online":
            return
        time.sleep(10)
    raise RunnerError("runner did not become SSM-online before timeout")


def ssh_base(ledger: dict[str, Any]) -> list[str]:
    key = state_dir() / "runner-ssh.key"
    if not key.is_file():
        raise RunnerError("private SSH key is missing from local state")
    known_hosts = state_dir() / "known_hosts"
    return [
        "ssh",
        "-i",
        str(key),
        "-o",
        f"UserKnownHostsFile={known_hosts}",
        "-o",
        "StrictHostKeyChecking=accept-new",
        "-o",
        "ConnectTimeout=10",
        f"ubuntu@{ledger['public_ipv4']}",
    ]


def command_wait_ready(_: argparse.Namespace) -> None:
    ledger = read_json("ledger.json")
    instance_id = resource(ledger, "instance")
    if not instance_id:
        raise RunnerError("ledger has no instance")
    if ledger["access_method"] == "ssm":
        wait_for_ssm(instance_id)
        deadline = time.monotonic() + 900
        while time.monotonic() < deadline:
            command_id = send_ssm(instance_id, "test -f /var/lib/sentia-runner/ready")
            if wait_ssm_command(instance_id, command_id, quiet=True):
                print("Runner is SSM-online and dependency bootstrap is complete.")
                return
            time.sleep(15)
        raise RunnerError("runner bootstrap did not complete before timeout")
    deadline = time.monotonic() + 900
    while time.monotonic() < deadline:
        proc = subprocess.run(
            [*ssh_base(ledger), "test -f /var/lib/sentia-runner/ready"],
            text=True,
            capture_output=True,
        )
        if proc.returncode == 0:
            print("Runner SSH is responsive and dependency bootstrap is complete.")
            return
        time.sleep(10)
    raise RunnerError("runner SSH/bootstrap did not become ready before timeout")


def send_ssm(instance_id: str, command: str) -> str:
    result = aws(
        [
            "ssm",
            "send-command",
            "--region",
            REGION,
            "--instance-ids",
            instance_id,
            "--document-name",
            "AWS-RunShellScript",
            "--parameters",
            json.dumps({"commands": [command]}, separators=(",", ":")),
            "--timeout-seconds",
            "3600",
        ]
    )
    return result["Command"]["CommandId"]


def wait_ssm_command(
    instance_id: str, command_id: str, *, quiet: bool = False, timeout: int = 3600
) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        proc = aws(
            [
                "ssm",
                "get-command-invocation",
                "--region",
                REGION,
                "--command-id",
                command_id,
                "--instance-id",
                instance_id,
            ],
            check=False,
        )
        if proc.returncode:
            if error_code(proc.stderr) == "InvocationDoesNotExist":
                time.sleep(3)
                continue
            raise RunnerError(
                f"SSM command status failed ({error_code(proc.stderr)})"
            )
        result = json.loads(proc.stdout)
        status = result.get("Status")
        if status == "Success":
            return True
        if status in {"Cancelled", "TimedOut", "Failed", "Cancelling"}:
            if not quiet:
                stderr = result.get("StandardErrorContent", "").strip()
                safe_tail = "\n".join(stderr.splitlines()[-12:])
                if safe_tail:
                    print(safe_tail, file=sys.stderr)
            return False
        time.sleep(5)
    return False


def command_upload_and_prove(args: argparse.Namespace) -> None:
    ledger = read_json("ledger.json")
    instance_id = resource(ledger, "instance")
    if not instance_id:
        raise RunnerError("ledger has no instance")
    script = Path(args.script).resolve()
    if not script.is_file():
        raise RunnerError("nested proof script does not exist")
    payload = __import__("base64").b64encode(script.read_bytes()).decode()
    remote = (
        "set -e; install -d -m 0755 /opt/sentia/bin; "
        f"printf '%s' '{payload}' | base64 -d >/opt/sentia/bin/prove-nested-kvm; "
        "chmod 0755 /opt/sentia/bin/prove-nested-kvm; "
        "/opt/sentia/bin/prove-nested-kvm"
    )
    if ledger["access_method"] == "ssm":
        wait_for_ssm(instance_id)
        command_id = send_ssm(instance_id, remote)
        if not wait_ssm_command(instance_id, command_id, timeout=3600):
            raise RunnerError("nested-KVM proof failed on the runner")
    else:
        proc = subprocess.run(
            [*ssh_base(ledger), "sudo", "bash", "-s"],
            input=script.read_text(),
            text=True,
            capture_output=True,
            timeout=3600,
        )
        if proc.returncode:
            safe_tail = "\n".join(proc.stderr.splitlines()[-12:])
            if safe_tail:
                print(safe_tail, file=sys.stderr)
            raise RunnerError("nested-KVM proof failed on the runner")
    ledger["nested_kvm_proof_at"] = iso_time(now())
    ledger["status"] = "validated"
    write_json("ledger.json", ledger)
    print("/dev/kvm, accelerated nested guest execution, guest CPU query, and")
    print("authenticated loopback-only guest SSH were validated successfully.")


def command_shell(_: argparse.Namespace) -> None:
    ledger = read_json("ledger.json")
    instance_id = resource(ledger, "instance")
    if not instance_id:
        raise RunnerError("ledger has no instance")
    if ledger["access_method"] == "ssm":
        os.execvp(
            "aws",
            [
                "aws",
                "ssm",
                "start-session",
                "--region",
                REGION,
                "--target",
                instance_id,
            ],
        )
    os.execvp("ssh", ssh_base(ledger))


def elapsed_hours(ledger: dict[str, Any]) -> Decimal:
    started = ledger.get("started_at", ledger["created_at"])
    start = dt.datetime.fromisoformat(started.replace("Z", "+00:00"))
    seconds = max(0, (now() - start).total_seconds())
    return Decimal(str(seconds)) / Decimal(3600)


def estimated_current_cost(
    ledger: dict[str, Any], preflight: dict[str, Any], cost: dict[str, Any]
) -> Decimal:
    hours = min(elapsed_hours(ledger), Decimal(cost["maximum_runtime_hours"]))
    compute = Decimal(preflight["compute_hourly_usd"]) * hours
    ebs = (
        Decimal(preflight["gp3_gb_month_usd"])
        * Decimal(cost["volume_gib"])
        * hours
        / Decimal(730)
    )
    ipv4 = Decimal(preflight["public_ipv4_hourly_usd"]) * hours
    transfer = Decimal(cost["transfer_reserved_usd"])
    return (
        (compute + ebs + ipv4 + transfer) * Decimal(cost["cost_multiplier"])
        + Decimal(cost["safety_and_cleanup_reserve_usd"])
    )


def command_status(_: argparse.Namespace) -> None:
    ledger = read_json("ledger.json")
    preflight = read_json("preflight.json")
    cost = read_json("budget.json")
    estimate = estimated_current_cost(ledger, preflight, cost)
    expiry = dt.datetime.fromisoformat(ledger["expires_at"].replace("Z", "+00:00"))
    remaining = max(0, int((expiry - now()).total_seconds()))
    print(f"Runner ledger status: {ledger['status']}")
    print(f"Access: {ledger['access_method']}; remaining hard-expiry time: {remaining // 60} min")
    print(
        f"Conservative current estimate including transfer/cleanup reserves: "
        f"${estimate:.2f} / ${BUDGET_USD}"
    )
    print("Private resource identifiers remain in the external ledger.")


def mark_deleted(ledger: dict[str, Any], kind: str, identifier: str) -> None:
    for item in ledger.get("resources", []):
        if item["kind"] == kind and item["id"] == identifier:
            item["deleted_at"] = iso_time(now())
    write_json("ledger.json", ledger)


def cleanup(ledger: dict[str, Any], *, quiet: bool = False) -> None:
    instance_id = resource(ledger, "instance")
    if instance_id:
        described = aws(
            [
                "ec2",
                "describe-instances",
                "--region",
                REGION,
                "--instance-ids",
                instance_id,
            ],
            check=False,
        )
        if described.returncode == 0:
            data = json.loads(described.stdout)
            instance = data["Reservations"][0]["Instances"][0]
            state = instance["State"]["Name"]
            if not expected_tags(instance, ledger):
                raise RunnerError("refusing teardown: instance ownership tags do not match")
            if state != "terminated":
                aws(
                    [
                        "ec2",
                        "terminate-instances",
                        "--region",
                        REGION,
                        "--instance-ids",
                        instance_id,
                    ]
                )
                aws(
                    [
                        "ec2",
                        "wait",
                        "instance-terminated",
                        "--region",
                        REGION,
                        "--instance-ids",
                        instance_id,
                    ],
                    json_output=False,
                )
            mark_deleted(ledger, "instance", instance_id)
        elif error_code(described.stderr) != "InvalidInstanceID.NotFound":
            raise RunnerError(
                f"could not inspect recorded instance ({error_code(described.stderr)})"
            )

    for item in ledger.get("resources", []):
        if item["kind"] == "volume" and not item.get("deleted_at"):
            result = aws(
                [
                    "ec2",
                    "describe-volumes",
                    "--region",
                    REGION,
                    "--volume-ids",
                    item["id"],
                ],
                check=False,
            )
            if result.returncode == 0:
                volume = json.loads(result.stdout)["Volumes"][0]
                actual = {tag["Key"]: tag["Value"] for tag in volume.get("Tags", [])}
                if (
                    actual.get("ManagedBy") != PROJECT_TAGS["ManagedBy"]
                    or actual.get("SentiaRunId") != ledger["run_id"]
                ):
                    raise RunnerError(
                        "refusing teardown: volume ownership tags do not match"
                    )
                if volume["State"] == "available":
                    aws(
                        [
                            "ec2",
                            "delete-volume",
                            "--region",
                            REGION,
                            "--volume-id",
                            item["id"],
                        ]
                    )
            mark_deleted(ledger, "volume", item["id"])

    key_name = resource(ledger, "key-pair")
    if key_name:
        aws(
            [
                "ec2",
                "delete-key-pair",
                "--region",
                REGION,
                "--key-name",
                key_name,
            ],
            check=False,
        )
        key_path = state_dir() / "runner-ssh.key"
        if key_path.exists():
            key_path.unlink()
        mark_deleted(ledger, "key-pair", key_name)

    group_id = resource(ledger, "security-group")
    if group_id:
        for attempt in range(12):
            result = aws(
                [
                    "ec2",
                    "delete-security-group",
                    "--region",
                    REGION,
                    "--group-id",
                    group_id,
                ],
                check=False,
            )
            if result.returncode == 0 or error_code(result.stderr) == "InvalidGroup.NotFound":
                mark_deleted(ledger, "security-group", group_id)
                break
            if error_code(result.stderr) != "DependencyViolation" or attempt == 11:
                raise RunnerError(
                    f"recorded security-group deletion failed ({error_code(result.stderr)})"
                )
            time.sleep(10)

    ledger["status"] = "terminated"
    ledger["terminated_at"] = iso_time(now())
    write_json("ledger.json", ledger)
    if not quiet:
        print("Recorded Sentia compute, volumes, key material, and security group cleaned up.")


def command_cleanup(_: argparse.Namespace) -> None:
    with operation_lock():
        cleanup(read_json("ledger.json"))


def command_reconcile(_: argparse.Namespace) -> None:
    ledger = read_json("ledger.json")
    active = aws(
        [
            "ec2",
            "describe-instances",
            "--region",
            REGION,
            "--filters",
            "Name=tag:SentiaProject,Values=Sentia",
            "Name=tag:SentiaPurpose,Values=vm-test-runner",
            "Name=instance-state-name,Values=pending,running,stopping,stopped",
        ]
    ).get("Reservations", [])
    instances = [
        instance for reservation in active for instance in reservation.get("Instances", [])
    ]
    recorded = resource(ledger, "instance")
    matching = sum(
        1
        for instance in instances
        if instance["InstanceId"] == recorded and expected_tags(instance, ledger)
    )
    unrecorded = len(instances) - matching
    print(
        f"Reconciliation: {matching} active ledger-matching runner; "
        f"{unrecorded} active tagged runner(s) not eligible for automatic deletion."
    )
    if unrecorded:
        raise RunnerError("manual review required; cleanup will not touch unrecorded resources")


def command_guard(args: argparse.Namespace) -> None:
    while True:
        try:
            with operation_lock():
                ledger = read_json("ledger.json")
                if ledger.get("status") == "terminated":
                    return
                preflight = read_json("preflight.json")
                cost = read_json("budget.json")
                estimate = estimated_current_cost(ledger, preflight, cost)
                expiry = dt.datetime.fromisoformat(
                    ledger["expires_at"].replace("Z", "+00:00")
                )
                if now() >= expiry or estimate >= GUARD_THRESHOLD_USD:
                    cleanup(ledger)
                    return
        except RunnerError as exc:
            print(f"cost guard stopped safely: {exc}", file=sys.stderr)
            return
        if args.once:
            return
        time.sleep(args.interval)


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    commands = result.add_subparsers(dest="command", required=True)
    commands.add_parser("preflight").set_defaults(function=command_preflight)
    provision = commands.add_parser("provision")
    provision.add_argument("--hours", type=Decimal, default=DEFAULT_RUNTIME_HOURS)
    provision.add_argument("--volume-gib", type=int, default=DEFAULT_VOLUME_GIB)
    provision.set_defaults(function=command_provision)
    commands.add_parser("wait-ready").set_defaults(function=command_wait_ready)
    proof = commands.add_parser("prove-nested-kvm")
    proof.add_argument(
        "--script",
        default=str(Path(__file__).with_name("prove-nested-kvm.sh")),
    )
    proof.set_defaults(function=command_upload_and_prove)
    commands.add_parser("shell").set_defaults(function=command_shell)
    commands.add_parser("status").set_defaults(function=command_status)
    commands.add_parser("cleanup").set_defaults(function=command_cleanup)
    commands.add_parser("reconcile").set_defaults(function=command_reconcile)
    guard = commands.add_parser("guard")
    guard.add_argument("--interval", type=int, default=300)
    guard.add_argument("--once", action="store_true")
    guard.set_defaults(function=command_guard)
    return result


def main() -> int:
    try:
        args = parser().parse_args()
        args.function(args)
        return 0
    except (RunnerError, ValueError, OSError, subprocess.SubprocessError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
