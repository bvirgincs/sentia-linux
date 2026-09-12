#!/usr/bin/env python3
"""Boot the Sentia live ISO and run acceptance checks inside the guest.

Serial-console log scraping cannot prove much: a message can be missing because
the boot failed, because output was redirected, or because systemd chose not to
print it. This harness instead logs into the booted guest over the serial
console and asks the running system directly.

The checks themselves are delivered on a separate read-only ISO, so no test hook
is shipped inside the Sentia image.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import time
from pathlib import Path

CHECK_LINE = re.compile(r"^CHECK\s+(?P<id>\S+)\s+(?P<status>PASS|FAIL)\s*(?P<detail>.*)$")
ANSI = re.compile(r"\x1b\[[0-9;?]*[a-zA-Z]|\x1b[=>]")

OVMF_CODE_CANDIDATES = (
    "/usr/share/OVMF/OVMF_CODE_4M.fd",
    "/usr/share/OVMF/OVMF_CODE.fd",
    "/usr/share/ovmf/OVMF.fd",
)
OVMF_VARS_CANDIDATES = (
    "/usr/share/OVMF/OVMF_VARS_4M.fd",
    "/usr/share/OVMF/OVMF_VARS.fd",
)


class ProbeError(RuntimeError):
    pass


def first_existing(candidates: tuple[str, ...], what: str) -> Path:
    for candidate in candidates:
        path = Path(candidate)
        if os.access(path, os.R_OK):
            return path
    raise ProbeError(f"no readable {what} found; install the ovmf package")


def build_check_iso(source: Path, output: Path) -> None:
    """Package the check script into a tiny ISO the guest can mount."""
    staging = output.parent / "checkiso"
    if staging.exists():
        shutil.rmtree(staging)
    staging.mkdir(parents=True)
    shutil.copy2(source, staging / "checks.sh")
    tool = shutil.which("xorrisofs") or shutil.which("genisoimage")
    if tool is None:
        raise ProbeError("xorrisofs or genisoimage is required to build the check ISO")
    subprocess.run(
        [tool, "-quiet", "-V", "SENTIACHECK", "-J", "-r", "-o", str(output), str(staging)],
        check=True,
        stdout=subprocess.DEVNULL,
    )


class SerialSession:
    """A line-oriented conversation with a guest getty over a Unix socket."""

    def __init__(self, connection: socket.socket, log_path: Path) -> None:
        self._connection = connection
        self._connection.settimeout(1.0)
        self._log = log_path.open("wb")
        self._buffer = ""

    def close(self) -> None:
        self._log.close()

    @property
    def transcript(self) -> str:
        return self._buffer

    def pump(self, seconds: float) -> None:
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            try:
                chunk = self._connection.recv(8192)
            except socket.timeout:
                continue
            except OSError as error:
                raise ProbeError(f"serial connection lost: {error}") from error
            if not chunk:
                raise ProbeError("guest closed the serial connection")
            self._log.write(chunk)
            self._log.flush()
            self._buffer += ANSI.sub("", chunk.decode("utf-8", "replace")).replace("\r", "")

    def wait_for(self, needle: str, timeout: float) -> bool:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if needle in self._buffer:
                return True
            self.pump(2)
        return needle in self._buffer

    def send(self, line: str) -> None:
        self._connection.sendall(line.encode() + b"\n")


def run_probe(args: argparse.Namespace) -> int:
    iso_path = Path(args.iso).resolve()
    if not iso_path.is_file():
        raise ProbeError(f"ISO not found: {iso_path}")

    run_dir = Path(args.run_dir).resolve()
    if run_dir.exists():
        shutil.rmtree(run_dir)
    run_dir.mkdir(parents=True)

    ovmf_code = first_existing(OVMF_CODE_CANDIDATES, "OVMF firmware code image")
    ovmf_vars = first_existing(OVMF_VARS_CANDIDATES, "OVMF variable store template")
    vars_copy = run_dir / "OVMF_VARS.fd"
    shutil.copy2(ovmf_vars, vars_copy)

    check_iso = run_dir / "sentia-checks.iso"
    build_check_iso(Path(args.checks).resolve(), check_iso)

    accel, cpu_model = ("kvm", "host") if os.access("/dev/kvm", os.W_OK) else ("tcg", "max")

    socket_path = run_dir / "serial.sock"
    server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    server.bind(str(socket_path))
    server.listen(1)
    server.settimeout(60)

    command = [
        "qemu-system-x86_64",
        "-machine", f"q35,accel={accel}",
        "-cpu", cpu_model,
        "-smp", str(args.cpus),
        "-m", str(args.memory_mb),
        "-name", "sentia-live-probe",
        "-display", "none",
        "-monitor", "none",
        "-no-reboot",
        "-drive", f"if=pflash,format=raw,readonly=on,file={ovmf_code}",
        "-drive", f"if=pflash,format=raw,file={vars_copy}",
        "-drive", f"file={iso_path},media=cdrom,readonly=on",
        "-drive", f"file={check_iso},media=cdrom,readonly=on",
        "-serial", f"unix:{socket_path}",
    ]
    if args.offline:
        command += ["-nic", "none"]

    print(f"live probe: accel={accel} memory={args.memory_mb}MiB offline={args.offline}", flush=True)
    qemu = subprocess.Popen(command)
    session: SerialSession | None = None
    results: list[dict[str, str]] = []
    failure: str | None = None
    try:
        connection, _ = server.accept()
        session = SerialSession(connection, run_dir / "serial.log")

        if not session.wait_for("login:", args.boot_timeout):
            raise ProbeError(
                f"no login prompt on the serial console within {args.boot_timeout}s"
            )
        print("live probe: guest reached a login prompt", flush=True)

        session.send(args.username)
        time.sleep(1.5)
        session.send(args.password)
        if not session.wait_for("$", 60):
            raise ProbeError("serial login did not produce a shell prompt")

        session.send("sudo mkdir -p /mnt/sentia-checks")
        time.sleep(1)
        session.send(
            "sudo mount -o ro -L SENTIACHECK /mnt/sentia-checks "
            "|| sudo mount -o ro /dev/sr1 /mnt/sentia-checks"
        )
        time.sleep(2)
        session.send("bash /mnt/sentia-checks/checks.sh")

        if not session.wait_for("SENTIA_CHECKS_COMPLETE", args.check_timeout):
            raise ProbeError(
                f"guest checks did not complete within {args.check_timeout}s"
            )
        session.pump(2)
    except ProbeError as error:
        failure = str(error)
    finally:
        if session is not None:
            transcript = session.transcript
            session.close()
        else:
            transcript = ""
        qemu.terminate()
        try:
            qemu.wait(timeout=20)
        except subprocess.TimeoutExpired:
            qemu.kill()
        server.close()
        socket_path.unlink(missing_ok=True)

    for line in transcript.splitlines():
        match = CHECK_LINE.match(line.strip())
        if match is None:
            continue
        # The guest echoes the command line itself; keep only real results.
        if match.group("id").startswith("<"):
            continue
        results.append(
            {
                "id": match.group("id"),
                "status": match.group("status"),
                "detail": match.group("detail").strip(),
            }
        )

    seen: dict[str, dict[str, str]] = {}
    for result in results:
        seen[result["id"]] = result
    results = [seen[key] for key in sorted(seen)]

    failed = [result for result in results if result["status"] == "FAIL"]
    report = {
        "iso": str(iso_path),
        "accel": accel,
        "memory_mb": args.memory_mb,
        "offline": args.offline,
        "checks": results,
        "failed": [result["id"] for result in failed],
        "error": failure,
    }
    report_path = run_dir / "live-probe.json"
    report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")

    for result in results:
        print(f"  {result['status']:4} {result['id']}: {result['detail']}", flush=True)
    print(f"live probe report: {report_path}", flush=True)

    if failure is not None:
        print(f"ERROR: {failure}", file=sys.stderr)
        print("--- last 60 serial lines ---", file=sys.stderr)
        print("\n".join(transcript.splitlines()[-60:]), file=sys.stderr)
        return 2
    if not results:
        print("ERROR: the guest produced no check results", file=sys.stderr)
        return 2
    if failed:
        print(f"ERROR: {len(failed)} live check(s) failed", file=sys.stderr)
        return 1
    print(f"live probe: all {len(results)} checks passed", flush=True)
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--iso", required=True)
    parser.add_argument("--run-dir", required=True)
    parser.add_argument(
        "--checks",
        default=str(Path(__file__).resolve().parent / "checks" / "live_checks.sh"),
    )
    parser.add_argument("--username", default="user")
    parser.add_argument("--password", default="live")
    parser.add_argument("--cpus", type=int, default=4)
    parser.add_argument("--memory-mb", type=int, default=4096)
    parser.add_argument("--boot-timeout", type=float, default=float(os.environ.get("SENTIA_ISO_BOOT_TIMEOUT", 300)))
    parser.add_argument("--check-timeout", type=float, default=240.0)
    parser.add_argument(
        "--offline",
        action="store_true",
        help="run the guest with no network device at all",
    )
    args = parser.parse_args()
    try:
        return run_probe(args)
    except ProbeError as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
