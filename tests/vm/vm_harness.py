#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Headless QEMU/OVMF/QMP harness for real Sentia live and install testing."""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import re
import shutil
import socket
import stat
import subprocess
import sys
import time
from pathlib import Path
from typing import Any, Iterable

UTC = dt.timezone.utc
QCODE = {
    " ": "spc",
    "\n": "ret",
    "\t": "tab",
    "-": "minus",
    "=": "equal",
    ".": "dot",
    ",": "comma",
    "/": "slash",
    ";": "semicolon",
    "'": "apostrophe",
}
SHIFTED = {
    "_": "minus",
    "+": "equal",
    ":": "semicolon",
    '"': "apostrophe",
    "?": "slash",
    "<": "comma",
    ">": "dot",
    "!": "1",
    "@": "2",
    "#": "3",
    "$": "4",
    "%": "5",
    "^": "6",
    "&": "7",
    "*": "8",
    "(": "9",
    ")": "0",
}


class HarnessError(RuntimeError):
    pass


def timestamp() -> str:
    return (
        dt.datetime.now(UTC)
        .replace(microsecond=0)
        .isoformat()
        .replace("+00:00", "Z")
    )


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def regular_image(path: Path, *, must_exist: bool = True) -> Path:
    resolved = path.expanduser().resolve()
    if str(resolved).startswith("/dev/"):
        raise HarnessError("host block devices are forbidden")
    if must_exist:
        info = resolved.stat()
        if stat.S_ISBLK(info.st_mode) or not stat.S_ISREG(info.st_mode):
            raise HarnessError(f"image is not a regular file: {resolved}")
    return resolved


def executable(name: str) -> str:
    result = shutil.which(name)
    if not result:
        raise HarnessError(f"required executable is missing: {name}")
    return result


def firmware_paths(code: str | None, variables: str | None) -> tuple[Path, Path]:
    code_candidates = [
        Path(code) if code else None,
        Path("/usr/share/OVMF/OVMF_CODE_4M.fd"),
        Path("/usr/share/OVMF/OVMF_CODE.fd"),
    ]
    vars_candidates = [
        Path(variables) if variables else None,
        Path("/usr/share/OVMF/OVMF_VARS_4M.fd"),
        Path("/usr/share/OVMF/OVMF_VARS.fd"),
    ]
    selected_code = next(
        (item.resolve() for item in code_candidates if item and item.is_file()), None
    )
    selected_vars = next(
        (item.resolve() for item in vars_candidates if item and item.is_file()), None
    )
    if not selected_code or not selected_vars:
        raise HarnessError("readable OVMF CODE and VARS firmware files are required")
    return selected_code, selected_vars


def load_scenario(path: Path) -> dict[str, Any]:
    data = json.loads(path.read_text())
    if data.get("schema_version") != 1 or not isinstance(data.get("steps"), list):
        raise HarnessError("scenario must have schema_version 1 and a steps array")
    allowed = {
        "wait",
        "screenshot",
        "click",
        "key",
        "text",
        "wait_serial",
        "powerdown",
        "quit",
    }
    for index, step in enumerate(data["steps"]):
        if not isinstance(step, dict) or step.get("action") not in allowed:
            raise HarnessError(f"unsupported scenario step {index}")
    return data


class QMP:
    def __init__(self, path: Path, timeout: float = 30.0):
        self.socket = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        deadline = time.monotonic() + timeout
        while True:
            try:
                self.socket.connect(str(path))
                break
            except (FileNotFoundError, ConnectionRefusedError):
                if time.monotonic() >= deadline:
                    raise HarnessError("QMP socket was not ready before timeout")
                time.sleep(0.2)
        self.stream = self.socket.makefile("rwb")
        greeting = self._read()
        if "QMP" not in greeting:
            raise HarnessError("invalid QMP greeting")
        self.execute("qmp_capabilities")

    def _read(self) -> dict[str, Any]:
        while True:
            line = self.stream.readline()
            if not line:
                raise HarnessError("QMP connection closed")
            value = json.loads(line)
            if "event" not in value:
                return value

    def execute(self, name: str, arguments: dict[str, Any] | None = None) -> Any:
        payload: dict[str, Any] = {"execute": name}
        if arguments:
            payload["arguments"] = arguments
        self.stream.write(json.dumps(payload).encode() + b"\r\n")
        self.stream.flush()
        reply = self._read()
        if "error" in reply:
            raise HarnessError(f"QMP {name} failed: {reply['error'].get('class', 'error')}")
        return reply.get("return")

    def close(self) -> None:
        self.stream.close()
        self.socket.close()


def key_events(qcode: str, shifted: bool = False) -> list[dict[str, Any]]:
    if not re.fullmatch(r"[a-z0-9_-]+", qcode):
        raise HarnessError(f"unsafe or unsupported QMP qcode: {qcode}")
    result: list[dict[str, Any]] = []
    if shifted:
        result.append(
            {"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": "shift"}}}
        )
    result.extend(
        [
            {"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": qcode}}},
            {"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": qcode}}},
        ]
    )
    if shifted:
        result.append(
            {"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": "shift"}}}
        )
    return result


def character_events(character: str) -> list[dict[str, Any]]:
    if "a" <= character <= "z" or "0" <= character <= "9":
        return key_events(character)
    if "A" <= character <= "Z":
        return key_events(character.lower(), shifted=True)
    if character in QCODE:
        return key_events(QCODE[character])
    if character in SHIFTED:
        return key_events(SHIFTED[character], shifted=True)
    raise HarnessError(f"unsupported QMP text character: U+{ord(character):04X}")


def send_text(qmp: QMP, text: str, key_delay: float) -> None:
    for character in text:
        qmp.execute("input-send-event", {"events": character_events(character)})
        time.sleep(key_delay)


def serial_matches(path: Path, pattern: str) -> bool:
    return bool(re.search(pattern, path.read_text(errors="replace"), re.MULTILINE))


def run_scenario(
    qmp: QMP,
    scenario: dict[str, Any],
    run_dir: Path,
    serial: Path,
) -> None:
    for index, step in enumerate(scenario["steps"]):
        action = step["action"]
        if action == "wait":
            seconds = float(step["seconds"])
            if seconds < 0 or seconds > 3600:
                raise HarnessError(f"invalid wait at step {index}")
            time.sleep(seconds)
        elif action == "screenshot":
            name = step.get("name", f"step-{index:03d}.ppm")
            if not re.fullmatch(r"[A-Za-z0-9_.-]+\.ppm", name):
                raise HarnessError(f"invalid screenshot name at step {index}")
            qmp.execute("screendump", {"filename": str(run_dir / name)})
        elif action == "click":
            x, y = int(step["x"]), int(step["y"])
            if not 0 <= x <= 32767 or not 0 <= y <= 32767:
                raise HarnessError("QMP absolute coordinates must be 0..32767")
            button = step.get("button", "left")
            if button not in {"left", "middle", "right"}:
                raise HarnessError("invalid pointer button")
            qmp.execute(
                "input-send-event",
                {
                    "events": [
                        {"type": "abs", "data": {"axis": "x", "value": x}},
                        {"type": "abs", "data": {"axis": "y", "value": y}},
                        {"type": "btn", "data": {"down": True, "button": button}},
                        {"type": "btn", "data": {"down": False, "button": button}},
                    ]
                },
            )
        elif action == "key":
            qmp.execute(
                "input-send-event",
                {"events": key_events(str(step["qcode"]), bool(step.get("shift")))},
            )
        elif action == "text":
            send_text(qmp, str(step["text"]), float(step.get("key_delay", 0.04)))
        elif action == "wait_serial":
            deadline = time.monotonic() + int(step.get("timeout", 300))
            while time.monotonic() < deadline:
                if serial.is_file() and serial_matches(serial, str(step["regex"])):
                    break
                time.sleep(1)
            else:
                raise HarnessError(f"serial assertion timed out at step {index}")
        elif action == "powerdown":
            qmp.execute("system_powerdown")
        elif action == "quit":
            qmp.execute("quit")


def qemu_version(binary: str) -> str:
    return subprocess.run(
        [binary, "--version"], text=True, capture_output=True, check=True
    ).stdout.splitlines()[0]


def guest_probe(args: argparse.Namespace, run_dir: Path) -> None:
    if not args.test_ssh_key:
        return
    probe = Path(__file__).with_name("guest-probe.sh")
    key = regular_image(Path(args.test_ssh_key))
    known_hosts = run_dir / "guest-known-hosts"
    output = run_dir / "guest-probe.json"
    deadline = time.monotonic() + args.ssh_timeout
    command = [
        "ssh",
        "-i",
        str(key),
        "-p",
        str(args.ssh_port),
        "-o",
        "BatchMode=yes",
        "-o",
        "StrictHostKeyChecking=accept-new",
        "-o",
        f"UserKnownHostsFile={known_hosts}",
        "-o",
        "ConnectTimeout=5",
        f"{args.ssh_user}@127.0.0.1",
        "sh -s",
    ]
    last_error = ""
    while time.monotonic() < deadline:
        result = subprocess.run(
            command, input=probe.read_text(), text=True, capture_output=True
        )
        if result.returncode == 0:
            value = json.loads(result.stdout)
            if value.get("ssh_authenticated") is not True:
                raise HarnessError("guest probe did not confirm SSH authentication")
            output.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")
            return
        last_error = result.stderr
        time.sleep(5)
    (run_dir / "guest-ssh-last-error.log").write_text(last_error)
    raise HarnessError("test-only guest SSH hook was not responsive before timeout")


def qemu_command(
    args: argparse.Namespace,
    run_dir: Path,
    disk: Path,
    code: Path,
    variables: Path,
) -> list[str]:
    qemu = executable("qemu-system-x86_64")
    machine = f"q35,accel={args.accel}"
    command = [
        qemu,
        "-name",
        "sentia-vm-test",
        "-machine",
        machine,
        "-cpu",
        "host" if args.accel == "kvm" else "max",
        "-smp",
        str(args.cpus),
        "-m",
        str(args.memory_mib),
        "-no-reboot",
        "-display",
        "none",
        "-monitor",
        "none",
        "-serial",
        f"file:{run_dir / 'serial.log'}",
        "-qmp",
        f"unix:{run_dir / 'qmp.sock'},server=on,wait=off",
        "-drive",
        f"if=pflash,format=raw,readonly=on,file={code}",
        "-drive",
        f"if=pflash,format=raw,file={variables}",
        "-drive",
        f"if=none,id=sentia_disk,format=qcow2,cache=none,file={disk}",
        "-device",
        "virtio-blk-pci,drive=sentia_disk,bootindex=2",
        "-device",
        "qemu-xhci",
        "-device",
        "usb-tablet",
        "-device",
        "virtio-vga",
    ]
    network = "user,id=sentia_net"
    if args.test_ssh_key:
        network += f",hostfwd=tcp:127.0.0.1:{args.ssh_port}-:22"
    command += [
        "-netdev",
        network,
        "-device",
        "virtio-net-pci,netdev=sentia_net",
    ]
    if args.mode in {"boot-iso", "install"}:
        iso = regular_image(Path(args.iso))
        command += [
            "-drive",
            f"if=none,id=sentia_iso,media=cdrom,readonly=on,file={iso}",
            "-device",
            "ide-cd,drive=sentia_iso,bootindex=1",
        ]
    return command


def wait_process(process: subprocess.Popen[bytes], timeout: int) -> None:
    try:
        process.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        raise HarnessError("VM did not stop before the overall timeout")


def wait_serial_assertions(
    process: subprocess.Popen[bytes],
    serial: Path,
    patterns: Iterable[str],
    timeout: int,
) -> None:
    pending = set(patterns)
    deadline = time.monotonic() + timeout
    while pending and time.monotonic() < deadline:
        if process.poll() is not None:
            raise HarnessError("VM exited before required serial evidence appeared")
        if serial.is_file():
            pending = {pattern for pattern in pending if not serial_matches(serial, pattern)}
        if pending:
            time.sleep(1)
    if pending:
        raise HarnessError(
            "required serial evidence timed out: " + ", ".join(sorted(pending))
        )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("boot-iso", "install", "disk-boot"))
    parser.add_argument("--iso")
    parser.add_argument("--disk")
    parser.add_argument("--disk-size", default="48G")
    parser.add_argument("--artifact-root", default="artifacts/vm")
    parser.add_argument("--run-id")
    parser.add_argument("--scenario")
    parser.add_argument("--ovmf-code")
    parser.add_argument("--ovmf-vars")
    parser.add_argument("--accel", choices=("kvm", "tcg"), default="kvm")
    parser.add_argument("--cpus", type=int, default=4)
    parser.add_argument("--memory-mib", type=int, default=16384)
    parser.add_argument("--timeout", type=int, default=3600)
    parser.add_argument("--success-serial-regex", action="append", default=[])
    parser.add_argument("--image-kind", choices=("test", "production"), required=True)
    parser.add_argument("--test-ssh-key")
    parser.add_argument("--ssh-user")
    parser.add_argument("--ssh-port", type=int, default=2222)
    parser.add_argument("--ssh-timeout", type=int, default=600)
    parser.add_argument("--dry-run", action="store_true")
    return parser


def validate_args(args: argparse.Namespace) -> None:
    if args.mode in {"boot-iso", "install"} and not args.iso:
        raise HarnessError(f"{args.mode} requires --iso")
    if args.mode == "disk-boot" and not args.disk:
        raise HarnessError("disk-boot requires --disk")
    if args.mode == "install" and not args.scenario:
        raise HarnessError("install requires a real Calamares QMP --scenario")
    if args.image_kind == "production" and (args.test_ssh_key or args.ssh_user):
        raise HarnessError("production images forbid test-only SSH authentication hooks")
    if bool(args.test_ssh_key) != bool(args.ssh_user):
        raise HarnessError("--test-ssh-key and --ssh-user must be supplied together")
    if args.test_ssh_key and args.image_kind != "test":
        raise HarnessError("test-only SSH hooks require --image-kind test")
    if not args.success_serial_regex and not args.test_ssh_key:
        raise HarnessError(
            "at least one serial success assertion or authenticated test guest probe is required"
        )
    if not 1 <= args.cpus <= 8:
        raise HarnessError("--cpus must be between 1 and 8")
    if not 1024 <= args.memory_mib <= 24576:
        raise HarnessError("--memory-mib must be between 1024 and 24576")
    if not 1 <= args.ssh_port <= 65535:
        raise HarnessError("invalid SSH port")
    if args.accel == "kvm":
        kvm = Path("/dev/kvm")
        if not kvm.exists() or not os.access(kvm, os.R_OK | os.W_OK):
            raise HarnessError("KVM requested but /dev/kvm is not usable")


def main() -> int:
    manifest: dict[str, Any] = {"schema_version": 1, "status": "failed"}
    process: subprocess.Popen[bytes] | None = None
    qmp: QMP | None = None
    qemu_log = None
    run_dir: Path | None = None
    try:
        args = build_parser().parse_args()
        validate_args(args)
        root = Path(args.artifact_root).expanduser().resolve()
        root.mkdir(parents=True, exist_ok=True)
        run_id = args.run_id or dt.datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")
        if not re.fullmatch(r"[A-Za-z0-9_.-]+", run_id):
            raise HarnessError("run ID contains unsafe characters")
        run_dir = root / run_id
        run_dir.mkdir(mode=0o700)

        qemu_img = executable("qemu-img")
        code, source_vars = firmware_paths(args.ovmf_code, args.ovmf_vars)
        variables = run_dir / "OVMF_VARS.fd"
        shutil.copyfile(source_vars, variables)
        if args.mode == "disk-boot":
            disk = regular_image(Path(args.disk))
        elif args.disk:
            disk = Path(args.disk).expanduser().resolve()
            if disk.exists():
                raise HarnessError("install output disk already exists")
            if root not in disk.parents:
                raise HarnessError("new VM disks must be created under --artifact-root")
            disk.parent.mkdir(parents=True, exist_ok=True)
            subprocess.run(
                [qemu_img, "create", "-f", "qcow2", str(disk), args.disk_size],
                check=True,
            )
        else:
            disk = run_dir / "sentia.qcow2"
            subprocess.run(
                [qemu_img, "create", "-f", "qcow2", str(disk), args.disk_size],
                check=True,
            )

        inputs: dict[str, Any] = {
            "disk": {"path": str(disk), "sha256_before": digest(disk)}
        }
        if args.iso:
            iso = regular_image(Path(args.iso))
            inputs["iso"] = {"path": str(iso), "sha256": digest(iso)}
        scenario = load_scenario(Path(args.scenario).resolve()) if args.scenario else None
        command = qemu_command(args, run_dir, disk, code, variables)
        manifest.update(
            {
                "started_at": timestamp(),
                "mode": args.mode,
                "image_kind": args.image_kind,
                "inputs": inputs,
                "guest": {
                    "memory_mib": args.memory_mib,
                    "cpus": args.cpus,
                    "accel": args.accel,
                    "firmware": "OVMF",
                    "disk_format": "qcow2",
                    "network": "qemu-user",
                    "test_ssh_auth": bool(args.test_ssh_key),
                },
                "evidence": {
                    "serial": "serial.log",
                    "qemu": "qemu.log",
                    "qmp": "qmp.sock",
                    "guest_probe": (
                        "guest-probe.json" if args.test_ssh_key else None
                    ),
                },
                "qemu_version": qemu_version(command[0]),
            }
        )
        if args.dry_run:
            manifest["status"] = "not-run"
            manifest["reason"] = "explicit dry run; no acceptance claim"
            (run_dir / "evidence.json").write_text(
                json.dumps(manifest, indent=2, sort_keys=True) + "\n"
            )
            print(run_dir / "evidence.json")
            return 0

        qemu_log = (run_dir / "qemu.log").open("wb")
        process = subprocess.Popen(
            command, cwd=run_dir, stdout=qemu_log, stderr=subprocess.STDOUT
        )
        qmp = QMP(run_dir / "qmp.sock")
        if scenario:
            run_scenario(qmp, scenario, run_dir, run_dir / "serial.log")
        guest_probe(args, run_dir)
        wait_serial_assertions(
            process,
            run_dir / "serial.log",
            args.success_serial_regex,
            args.timeout,
        )

        if process.poll() is None:
            qmp.execute("system_powerdown")
            wait_process(process, min(args.timeout, 180))
        manifest["status"] = "passed"
        manifest["completed_at"] = timestamp()
        manifest["outputs"] = {
            "disk": {"path": str(disk), "sha256_after": digest(disk)}
        }
        (run_dir / "evidence.json").write_text(
            json.dumps(manifest, indent=2, sort_keys=True) + "\n"
        )
        print(run_dir / "evidence.json")
        return 0
    except Exception as exc:
        manifest["status"] = "failed"
        manifest["completed_at"] = timestamp()
        manifest["reason"] = str(exc)
        if run_dir:
            (run_dir / "evidence.json").write_text(
                json.dumps(manifest, indent=2, sort_keys=True) + "\n"
            )
        print(f"error: {exc}", file=sys.stderr)
        return 1
    finally:
        if qmp:
            try:
                qmp.close()
            except OSError:
                pass
        if process and process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
        if qemu_log:
            qemu_log.close()


if __name__ == "__main__":
    raise SystemExit(main())
