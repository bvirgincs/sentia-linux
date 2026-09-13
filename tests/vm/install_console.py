#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""An interactive console for the live installer, used to tune install_probe.

The installer is driven by pointer coordinates, and a wrong coordinate costs a
forty-minute test cycle to discover. This boots the same VM the install probe
boots, starts Calamares the same way, and then executes single commands from a
file so the widget positions can be read off real screenshots instead of
guessed. It installs nothing by itself.

    install_console.py --iso ISO --run-dir DIR --commands FILE

Each line appended to FILE is one of:

    click X Y          press the left button at screen pixel X, Y
    key QCODE [N]      send a QEMU key code, optionally N times
    shift QCODE        send a shifted QEMU key code
    text STRING        type a string
    shot NAME          write DIR/NAME.png
    sh COMMAND         run a command on the guest serial shell
    quit               shut the VM down and exit
"""
import argparse
import importlib.util
import struct
import sys
import time
import zlib
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("install_probe.py")
SPEC = importlib.util.spec_from_file_location("install_probe", MODULE_PATH)
assert SPEC and SPEC.loader
probe = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(probe)


def ppm_to_png(source: Path, target: Path) -> None:
    """Convert a QEMU screendump so the image can be read directly."""
    data = source.read_bytes()
    if data[:2] != b"P6":
        raise probe.ProbeError(f"not a binary PPM: {source}")
    fields: list[int] = []
    index = 2
    while len(fields) < 3:
        while data[index : index + 1].isspace():
            index += 1
        if data[index : index + 1] == b"#":
            while data[index : index + 1] != b"\n":
                index += 1
            continue
        end = index
        while not data[end : end + 1].isspace():
            end += 1
        fields.append(int(data[index:end]))
        index = end
    index += 1
    width, height, _ = fields
    pixels = data[index : index + width * height * 3]
    raw = b"".join(
        b"\x00" + pixels[row * width * 3 : (row + 1) * width * 3]
        for row in range(height)
    )

    def chunk(kind: bytes, payload: bytes) -> bytes:
        body = kind + payload
        return (
            struct.pack(">I", len(payload))
            + body
            + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)
        )

    target.write_bytes(
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw, 6))
        + chunk(b"IEND", b"")
    )


def execute(line: str, screen, guest, args, run_dir: Path) -> bool:
    """Run one console command. Returns False when the console should stop."""
    parts = line.split(maxsplit=1)
    verb = parts[0]
    rest = parts[1] if len(parts) > 1 else ""
    if verb == "quit":
        return False
    if verb == "click":
        x, y = (int(value) for value in rest.split())
        screen.click(x, y, args.width, args.height)
    elif verb == "key":
        fields = rest.split()
        screen.key(fields[0], repeat=int(fields[1]) if len(fields) > 1 else 1)
    elif verb == "shift":
        screen.key(rest.strip(), shift=True)
    elif verb == "text":
        screen.text(rest)
    elif verb == "shot":
        name = rest.strip() or "shot"
        ppm = screen.shot(name)
        png = run_dir / f"{name}.png"
        ppm_to_png(ppm, png)
        print(f"shot {png}", flush=True)
    elif verb == "sh":
        print(f"guest: {guest.run(rest, timeout=180)}", flush=True)
    else:
        print(f"unknown command: {line}", flush=True)
    return True


def run_console(args: argparse.Namespace) -> int:
    run_dir = Path(args.run_dir).resolve()
    run_dir.mkdir(parents=True, exist_ok=True)
    commands = Path(args.commands).resolve()
    commands.touch()

    process, server, session, guest, screen = probe.start_installer_session(
        args, run_dir
    )
    consumed = 0
    try:
        print("console ready", flush=True)
        while True:
            lines = [
                line.strip()
                for line in commands.read_text(encoding="utf-8").splitlines()
            ]
            if len(lines) > consumed:
                for line in lines[consumed:]:
                    consumed += 1
                    if not line or line.startswith("#"):
                        continue
                    print(f"> {line}", flush=True)
                    if not execute(line, screen, guest, args, run_dir):
                        return 0
            else:
                time.sleep(2)
    finally:
        session.close()
        probe.stop_qemu(process, server, run_dir / "install-serial.sock")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--iso", required=True)
    parser.add_argument("--run-dir", required=True)
    parser.add_argument("--commands", required=True)
    parser.add_argument("--disk-gb", type=int, default=30)
    parser.add_argument("--cpus", type=int, default=4)
    parser.add_argument("--memory-mb", type=int, default=8192)
    parser.add_argument("--boot-timeout", type=float, default=600.0)
    parser.add_argument("--desktop-timeout", type=float, default=600.0)
    parser.add_argument("--width", type=int, default=1024)
    parser.add_argument("--height", type=int, default=768)
    args = parser.parse_args(argv)
    try:
        return run_console(args)
    except probe.ProbeError as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
