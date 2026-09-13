#!/usr/bin/env python3
"""Install Sentia into a virtual disk with the real Calamares installer.

The live probe proves the image boots. This proves it installs: it boots the
live ISO with a blank disk attached, starts the shipped Calamares wrapper,
drives the actual graphical installer through QMP input events, and then boots
the resulting disk with the installation media detached.

Nothing here is shipped in the image. The installer is driven the way a person
drives it, through the keyboard and pointer, and every page is captured as a
screenshot so a failed run can be diagnosed without re-running it.
"""
from __future__ import annotations

import argparse
import json
import os
import shutil
import socket
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from live_probe import (  # noqa: E402  (path set above)
    CHECK_LINE,
    OVMF_CODE_CANDIDATES,
    OVMF_VARS_CANDIDATES,
    ProbeError,
    SerialSession,
    build_check_iso,
    first_existing,
)
from vm_harness import QMP, character_events, key_events  # noqa: E402

# The live account live-config creates. The installer runs from this session.
LIVE_USER = "user"
LIVE_PASSWORD = "live"

# The account the installer creates, used to prove the installed disk logs in.
INSTALLED_FULL_NAME = "Sentia Test"
INSTALLED_USER = "sentia"
INSTALLED_PASSWORD = "sentia-install-test"
INSTALLED_HOSTNAME = "sentia-installed"

CALAMARES_LOG = "/tmp/sentia-calamares.log"


class Screen:
    """QMP input and screenshots, with every action recorded."""

    def __init__(self, qmp: QMP, run_dir: Path) -> None:
        self._qmp = qmp
        self._run_dir = run_dir
        self._shots = 0

    def shot(self, name: str) -> Path:
        self._shots += 1
        path = self._run_dir / f"{self._shots:02d}-{name}.ppm"
        self._qmp.execute("screendump", {"filename": str(path)})
        return path

    def key(self, qcode: str, *, shift: bool = False, repeat: int = 1) -> None:
        for _ in range(repeat):
            self._qmp.execute(
                "input-send-event", {"events": key_events(qcode, shift)}
            )
            time.sleep(0.15)

    def text(self, value: str) -> None:
        for character in value:
            self._qmp.execute(
                "input-send-event", {"events": character_events(character)}
            )
            time.sleep(0.05)

    def click(self, x: int, y: int, width: int, height: int) -> None:
        # QMP absolute coordinates are a 0..32767 range over the whole screen.
        abs_x = int(x * 32767 / width)
        abs_y = int(y * 32767 / height)
        self._qmp.execute(
            "input-send-event",
            {
                "events": [
                    {"type": "abs", "data": {"axis": "x", "value": abs_x}},
                    {"type": "abs", "data": {"axis": "y", "value": abs_y}},
                ]
            },
        )
        time.sleep(0.3)
        # Press and release have to be separate events with time between them.
        # Sent as one batch they reach the guest in the same input frame: a
        # combo box still opens, because that happens on press, but a push
        # button never completes a click and Calamares sat on its first page.
        for down in (True, False):
            self._qmp.execute(
                "input-send-event",
                {"events": [{"type": "btn", "data": {"down": down, "button": "left"}}]},
            )
            time.sleep(0.2)
        time.sleep(0.4)


def split_marker(marker: str) -> str:
    """Type a marker the guest's terminal echo cannot reproduce verbatim."""
    return f"{marker[:-2]}''{marker[-2:]}"


class Guest:
    """A logged-in serial shell, used to observe rather than to install."""

    def __init__(self, session: SerialSession) -> None:
        self._session = session
        self._token = 0

    def run(self, command: str, timeout: float = 60.0) -> str:
        """Run a command and return its output, delimited by a unique token.

        Each marker is typed in two halves joined by an empty quoted string, so
        the guest's own echo of the command line never contains the marker the
        shell later prints. Without that, the delimiters matched the echoed
        command first and every command appeared to produce no output at all.
        """
        self._token += 1
        start = f"SENTIA_CMD_{self._token}_BEGIN"
        end = f"SENTIA_CMD_{self._token}_END"
        mark = len(self._session.transcript)
        self._session.send(
            f"echo {split_marker(start)}; {command}; echo {split_marker(end)}$?"
        )
        if not self._session.wait_for(end, timeout):
            raise ProbeError(f"guest command timed out: {command}")
        tail = self._session.transcript[mark:]
        body = tail.split(start, 1)[-1]
        body = body.split(end, 1)[0]
        return body.strip()

    def launch(self, command: str, timeout: float = 60.0) -> str:
        """Start a background command and return once the shell accepts it.

        A trailing ``&`` cannot simply be followed by ``;`` and the marker echo,
        because ``& ;`` is a bash syntax error and the end marker would never be
        printed. Braces make the background command a complete list of its own.
        """
        return self.run(f"{{ {command} }}", timeout)

    def wait_until(self, command: str, timeout: float, poll: float = 5.0) -> bool:
        """Poll a predicate command until it succeeds."""
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if self.run(f"({command}) >/dev/null 2>&1 && echo YES || echo NO") \
                    .endswith("YES"):
                return True
            time.sleep(poll)
        return False


def qemu_base(
    *,
    accel: str,
    cpu: str,
    cpus: int,
    memory_mb: int,
    ovmf_code: Path,
    vars_copy: Path,
    serial_socket: Path,
    qmp_socket: Path,
    width: int,
    height: int,
) -> list[str]:
    return [
        "qemu-system-x86_64",
        "-machine", f"q35,accel={accel}",
        "-cpu", cpu,
        "-smp", str(cpus),
        "-m", str(memory_mb),
        "-display", "none",
        "-monitor", "none",
        # The standard VGA device is the one whose default mode can be fixed
        # from the host, which is what makes pointer coordinates predictable.
        "-vga", "std",
        "-global", f"VGA.xres={width}",
        "-global", f"VGA.yres={height}",
        # An absolute pointing device: relative mouse movement cannot be
        # driven reliably from outside the guest.
        "-device", "virtio-tablet-pci",
        "-drive", f"if=pflash,format=raw,readonly=on,file={ovmf_code}",
        "-drive", f"if=pflash,format=raw,file={vars_copy}",
        "-serial", f"unix:{serial_socket}",
        "-qmp", f"unix:{qmp_socket},server=on,wait=off",
    ]


def start_qemu(command: list[str], serial_socket: Path, log: Path):
    server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    server.bind(str(serial_socket))
    server.listen(1)
    server.settimeout(120)
    process = subprocess.Popen(command)
    connection, _ = server.accept()
    return process, server, SerialSession(connection, log)


def stop_qemu(process, server, socket_path: Path) -> None:
    process.terminate()
    try:
        process.wait(timeout=30)
    except subprocess.TimeoutExpired:
        process.kill()
    server.close()
    socket_path.unlink(missing_ok=True)


def serial_login(session: SerialSession, timeout: float) -> None:
    if not session.wait_for("login:", timeout):
        raise ProbeError(f"no login prompt within {timeout}s")
    session.send(LIVE_USER)
    time.sleep(1.5)
    session.send(LIVE_PASSWORD)
    if not session.wait_for("$", 60):
        raise ProbeError("serial login did not produce a shell prompt")


def drive_calamares(screen: Screen, guest: Guest, args: argparse.Namespace) -> None:
    """Walk the six Calamares pages, then wait out the exec phase."""
    width, height = args.width, args.height

    screen.shot("welcome")
    # Every page but the partition page accepts the default and advances on the
    # Next button, which Calamares makes the default button.
    for page in ("locale", "keyboard", "partition"):
        screen.click(args.next_x, args.next_y, width, height)
        time.sleep(3)
        screen.shot(page)

    # The partition page deliberately preselects nothing, so the erase-disk
    # choice has to be made before Next becomes usable.
    screen.click(args.erase_x, args.erase_y, width, height)
    time.sleep(2)
    screen.shot("partition-erase")
    screen.click(args.next_x, args.next_y, width, height)
    time.sleep(3)
    screen.shot("users")

    # The users page: full name, then the login name, hostname and password
    # fields in tab order. Calamares derives the login name and hostname from
    # the full name, so both are overwritten explicitly.
    screen.click(args.users_x, args.users_y, width, height)
    screen.text(INSTALLED_FULL_NAME)
    time.sleep(1)
    screen.key("tab")
    screen.key("ctrl-a") if False else None
    screen.text(INSTALLED_USER)
    screen.key("tab")
    screen.text(INSTALLED_HOSTNAME)
    screen.key("tab")
    screen.text(INSTALLED_PASSWORD)
    screen.key("tab")
    screen.text(INSTALLED_PASSWORD)
    time.sleep(1)
    screen.shot("users-filled")

    screen.click(args.next_x, args.next_y, width, height)
    time.sleep(3)
    screen.shot("summary")
    screen.click(args.next_x, args.next_y, width, height)
    time.sleep(5)
    screen.shot("install-started")


def wait_for_install(guest: Guest, screen: Screen, timeout: float) -> None:
    """Wait on the target mount rather than on log wording.

    Calamares mounts the new system under /tmp/calamares-root-* for the whole
    exec phase and unmounts it in its last module, so the mount appearing and
    then disappearing is an exact, translation-independent progress signal.
    """
    mounted = "findmnt -n -o TARGET | grep -q calamares-root"
    alive = "pgrep -x calamares"
    deadline = time.monotonic() + timeout
    seen_mount = False
    last_shot = 0.0
    while time.monotonic() < deadline:
        state = guest.run(
            f"if {mounted}; then echo MOUNTED;"
            f" elif {alive} >/dev/null 2>&1; then echo RUNNING;"
            " else echo GONE; fi"
        )
        if state.endswith("MOUNTED"):
            seen_mount = True
        elif seen_mount and state.endswith("RUNNING"):
            screen.shot("install-finished")
            return
        elif state.endswith("GONE"):
            screen.shot("install-exited")
            tail = guest.run(f"tail -n 40 {CALAMARES_LOG}", timeout=120)
            raise ProbeError(
                "Calamares exited before the installation completed:\n" + tail
            )
        if time.monotonic() - last_shot > 180:
            screen.shot("installing")
            last_shot = time.monotonic()
        time.sleep(15)
    raise ProbeError(f"the installation did not finish within {timeout}s")


def create_disk(disk: Path, disk_gb: int) -> None:
    subprocess.run(
        ["qemu-img", "create", "-f", "qcow2", str(disk), f"{disk_gb}G"],
        check=True,
        stdout=subprocess.DEVNULL,
    )


def start_installer_session(args: argparse.Namespace, run_dir: Path):
    """Boot the live ISO and leave Calamares on screen, driven by nothing yet.

    Shared with install_console.py so the console tunes the exact environment
    the probe installs in.
    """
    disk = Path(getattr(args, "disk", "") or (run_dir / "sentia-installed.qcow2"))
    if not disk.exists():
        create_disk(disk, args.disk_gb)
    ovmf_code = first_existing(OVMF_CODE_CANDIDATES, "OVMF firmware code image")
    ovmf_vars = first_existing(OVMF_VARS_CANDIDATES, "OVMF variable store template")
    vars_copy = run_dir / "OVMF_VARS_install.fd"
    shutil.copy2(ovmf_vars, vars_copy)

    accel, cpu = ("kvm", "host") if os.access("/dev/kvm", os.W_OK) else ("tcg", "max")
    serial_socket = run_dir / "install-serial.sock"
    qmp_socket = run_dir / "install-qmp.sock"

    command = qemu_base(
        accel=accel,
        cpu=cpu,
        cpus=args.cpus,
        memory_mb=args.memory_mb,
        ovmf_code=ovmf_code,
        vars_copy=vars_copy,
        serial_socket=serial_socket,
        qmp_socket=qmp_socket,
        width=args.width,
        height=args.height,
    )
    command += [
        "-drive", f"file={args.iso},media=cdrom,readonly=on",
        "-drive", f"file={disk},format=qcow2,if=virtio,cache=unsafe",
    ]

    print(f"install: accel={accel} disk={disk}", flush=True)
    process, server, session = start_qemu(command, serial_socket, run_dir / "install-serial.log")
    try:
        serial_login(session, args.boot_timeout)
        guest = Guest(session)
        qmp = QMP(qmp_socket, timeout=60)
        screen = Screen(qmp, run_dir)

        if not guest.wait_until(
            "systemctl is-active graphical.target", args.desktop_timeout
        ):
            raise ProbeError("graphical.target never became active")
        if not guest.wait_until(
            f"loginctl show-user {LIVE_USER} --property=State | grep -q active",
            args.desktop_timeout,
        ):
            raise ProbeError("the live user never got an active session")
        screen.shot("desktop")

        # Run the shipped wrapper, not calamares directly, so the fstab
        # handling it performs is part of what is being tested. pkexec is the
        # only thing replaced: it needs an interactive agent.
        guest.launch(
            "sudo -n env DISPLAY=:0 "
            f"XAUTHORITY=/home/{LIVE_USER}/.Xauthority "
            "setsid /usr/libexec/sentia-live/run-calamares-root -d "
            f"> {CALAMARES_LOG} 2>&1 < /dev/null &"
        )
        if not guest.wait_until(f"grep -qa 'Calamares' {CALAMARES_LOG}", 180):
            raise ProbeError("Calamares did not start")
        time.sleep(15)
    except BaseException:
        session.close()
        stop_qemu(process, server, serial_socket)
        qmp_socket.unlink(missing_ok=True)
        raise
    return process, server, session, guest, screen


def install_phase(args: argparse.Namespace, run_dir: Path, disk: Path) -> dict:
    args.disk = disk
    serial_socket = run_dir / "install-serial.sock"
    qmp_socket = run_dir / "install-qmp.sock"
    process, server, session, guest, screen = start_installer_session(args, run_dir)
    try:
        drive_calamares(screen, guest, args)
        wait_for_install(guest, screen, args.install_timeout)

        log_copy = run_dir / "calamares.log"
        log_copy.write_text(guest.run(f"cat {CALAMARES_LOG}", timeout=180), encoding="utf-8")
        transcript = session.transcript
    finally:
        session.close()
        stop_qemu(process, server, serial_socket)
        qmp_socket.unlink(missing_ok=True)

    return {"phase": "install", "serial_lines": len(transcript.splitlines())}


def installed_boot_phase(args: argparse.Namespace, run_dir: Path, disk: Path) -> list[dict]:
    """Boot the installed disk with no media attached and run the checks."""
    ovmf_code = first_existing(OVMF_CODE_CANDIDATES, "OVMF firmware code image")
    ovmf_vars = first_existing(OVMF_VARS_CANDIDATES, "OVMF variable store template")
    vars_copy = run_dir / "OVMF_VARS_disk.fd"
    shutil.copy2(ovmf_vars, vars_copy)

    check_iso = run_dir / "sentia-installed-checks.iso"
    build_check_iso(Path(args.installed_checks).resolve(), check_iso)

    accel, cpu = ("kvm", "host") if os.access("/dev/kvm", os.W_OK) else ("tcg", "max")
    serial_socket = run_dir / "disk-serial.sock"
    qmp_socket = run_dir / "disk-qmp.sock"

    command = qemu_base(
        accel=accel,
        cpu=cpu,
        cpus=args.cpus,
        memory_mb=args.memory_mb,
        ovmf_code=ovmf_code,
        vars_copy=vars_copy,
        serial_socket=serial_socket,
        qmp_socket=qmp_socket,
        width=args.width,
        height=args.height,
    )
    # The installation media is gone: only the disk and the check ISO remain.
    command += [
        "-drive", f"file={disk},format=qcow2,if=virtio",
        "-drive", f"file={check_iso},media=cdrom,readonly=on",
    ]

    print("installed boot: media detached", flush=True)
    process, server, session = start_qemu(command, serial_socket, run_dir / "disk-serial.log")
    results: list[dict] = []
    try:
        if not session.wait_for("login:", args.boot_timeout):
            raise ProbeError("the installed disk did not reach a login prompt")
        session.send(INSTALLED_USER)
        time.sleep(1.5)
        session.send(INSTALLED_PASSWORD)
        if not session.wait_for("$", 60):
            raise ProbeError("the installed user could not log in")
        guest = Guest(session)
        guest.run("sudo -n mkdir -p /mnt/sentia-checks")
        guest.run(
            "sudo -n mount -o ro -L SENTIACHECK /mnt/sentia-checks "
            "|| sudo -n mount -o ro /dev/sr0 /mnt/sentia-checks"
        )
        session.send("bash /mnt/sentia-checks/checks.sh")
        if not session.wait_for("SENTIA_CHECKS_COMPLETE", args.check_timeout):
            raise ProbeError("the installed-system checks did not complete")
        session.pump(2)
        transcript = session.transcript
    finally:
        session.close()
        stop_qemu(process, server, serial_socket)
        qmp_socket.unlink(missing_ok=True)

    for line in transcript.splitlines():
        match = CHECK_LINE.match(line.strip())
        if match is None or match.group("id").startswith("<"):
            continue
        results.append(
            {
                "id": match.group("id"),
                "status": match.group("status"),
                "detail": match.group("detail").strip(),
            }
        )
    return results


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--iso", required=True)
    parser.add_argument("--run-dir", required=True)
    parser.add_argument(
        "--installed-checks",
        default=str(Path(__file__).resolve().parent / "checks" / "installed_checks.sh"),
    )
    parser.add_argument("--cpus", type=int, default=4)
    # Calamares requires 20 GiB and the unpacked image is about 6 GiB.
    parser.add_argument("--disk-gb", type=int, default=30)
    parser.add_argument("--memory-mb", type=int, default=8192)
    parser.add_argument("--boot-timeout", type=float, default=600.0)
    parser.add_argument("--desktop-timeout", type=float, default=600.0)
    parser.add_argument("--install-timeout", type=float, default=3600.0)
    parser.add_argument("--check-timeout", type=float, default=1500.0)
    parser.add_argument("--width", type=int, default=1024)
    parser.add_argument("--height", type=int, default=768)
    # Calamares centres an 800x520 window on the screen. These are the defaults
    # for 1024x768 and can be overridden without editing the driver.
    parser.add_argument("--next-x", type=int, default=782)
    parser.add_argument("--next-y", type=int, default=610)
    parser.add_argument("--erase-x", type=int, default=160)
    parser.add_argument("--erase-y", type=int, default=250)
    parser.add_argument("--users-x", type=int, default=420)
    parser.add_argument("--users-y", type=int, default=175)
    parser.add_argument("--skip-installed-boot", action="store_true")
    return parser


def main() -> int:
    args = build_parser().parse_args()
    run_dir = Path(args.run_dir).resolve()
    if run_dir.exists():
        shutil.rmtree(run_dir)
    run_dir.mkdir(parents=True)

    iso = Path(args.iso).resolve()
    if not iso.is_file():
        raise ProbeError(f"ISO not found: {iso}")
    args.iso = str(iso)

    disk = run_dir / "sentia-installed.qcow2"
    create_disk(disk, args.disk_gb)

    report: dict = {"iso": str(iso), "disk_gb": args.disk_gb, "error": None}
    try:
        report["install"] = install_phase(args, run_dir, disk)
        if args.skip_installed_boot:
            report["checks"] = []
        else:
            report["checks"] = installed_boot_phase(args, run_dir, disk)
    except ProbeError as error:
        report["error"] = str(error)

    failed = [check for check in report.get("checks", []) if check["status"] == "FAIL"]
    report["failed"] = [check["id"] for check in failed]
    report_path = run_dir / "install-probe.json"
    report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(f"install probe report: {report_path}", flush=True)

    for check in report.get("checks", []):
        print(f"  {check['status']:4} {check['id']}: {check['detail']}", flush=True)

    if report["error"]:
        print(f"ERROR: {report['error']}", file=sys.stderr)
        return 2
    if not args.skip_installed_boot and not report["checks"]:
        print("ERROR: the installed system produced no check results", file=sys.stderr)
        return 2
    if failed:
        print(f"ERROR: {len(failed)} installed-system check(s) failed", file=sys.stderr)
        return 1
    print("install probe: installation and installed-system checks passed", flush=True)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ProbeError as error:  # pragma: no cover - top-level reporting
        print(f"ERROR: {error}", file=sys.stderr)
        raise SystemExit(2) from error
