#!/usr/bin/env python3
"""Generate bounded command->package index from Debian Contents metadata."""

from __future__ import annotations

import argparse
import datetime as dt
import gzip
import hashlib
import json
from pathlib import Path
from typing import Dict, Iterable, List, Set, Tuple


DEFAULT_PREFIXES = ("/usr/bin/", "/bin/", "/usr/sbin/", "/sbin/")


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def open_contents(path: Path):
    if path.suffix == ".gz":
        return gzip.open(path, "rt", encoding="utf-8", errors="replace")
    return path.open("rt", encoding="utf-8", errors="replace")


def parse_packages(raw: str) -> List[str]:
    values: List[str] = []
    for token in raw.split(","):
        token = token.strip()
        if not token:
            continue
        values.append(token.rsplit("/", 1)[-1])
    return values


def parse_line(line: str) -> Tuple[str, List[str]] | None:
    stripped = line.strip()
    if not stripped:
        return None
    parts = stripped.split()
    if len(parts) < 2:
        return None
    file_path = parts[0]
    package_blob = parts[-1]
    packages = parse_packages(package_blob)
    if not packages:
        return None
    normalized_path = file_path if file_path.startswith("/") else f"/{file_path}"
    return normalized_path, packages


def build_index(
    contents_files: Iterable[Path],
    allowed_prefixes: Tuple[str, ...],
    max_commands: int,
    max_packages_per_command: int,
) -> Dict[str, Dict[str, Set[str]]]:
    commands: Dict[str, Dict[str, Set[str]]] = {}

    for contents_file in contents_files:
        with open_contents(contents_file) as handle:
            for line in handle:
                parsed = parse_line(line)
                if parsed is None:
                    continue

                full_path, packages = parsed
                if not full_path.startswith(allowed_prefixes):
                    continue

                command = Path(full_path).name
                if not command:
                    continue

                if command not in commands and len(commands) >= max_commands:
                    continue

                bucket = commands.setdefault(
                    command, {"paths": set(), "packages": set()}
                )
                bucket["paths"].add(full_path)
                for package in packages:
                    if len(bucket["packages"]) >= max_packages_per_command:
                        break
                    bucket["packages"].add(package)

    return commands


def render_index(
    commands: Dict[str, Dict[str, Set[str]]],
    contents_files: List[Path],
    suite: str,
    snapshot: str,
    source_uri: str,
    generated_at: str,
    max_commands: int,
    max_packages_per_command: int,
) -> dict:
    normalized_commands = {}
    for command in sorted(commands):
        normalized_commands[command] = {
            "paths": sorted(commands[command]["paths"]),
            "packages": sorted(commands[command]["packages"]),
        }

    provenance_files = []
    for path in contents_files:
        provenance_files.append(
            {
                "path": str(path),
                "sha256": sha256_file(path),
                "bytes": path.stat().st_size,
            }
        )

    return {
        "schema_version": "1.0",
        "generator": "sentia-command-index/1",
        "generated_at": generated_at,
        "provenance": {
            "suite": suite,
            "snapshot": snapshot,
            "source_uri": source_uri,
            "signed_contents_required": True,
            "contents_files": provenance_files,
        },
        "limits": {
            "max_commands": max_commands,
            "max_packages_per_command": max_packages_per_command,
        },
        "commands": normalized_commands,
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Generate bounded command->package index from Debian Contents files."
        )
    )
    parser.add_argument(
        "--contents",
        dest="contents",
        action="append",
        required=True,
        help="Path to Debian Contents file (plain text or .gz). Repeatable.",
    )
    parser.add_argument("--output", required=True, help="Output JSON file path.")
    parser.add_argument("--suite", required=True, help="Debian suite (e.g. trixie).")
    parser.add_argument(
        "--snapshot",
        required=True,
        help="Snapshot identifier or timestamp for provenance.",
    )
    parser.add_argument(
        "--source-uri",
        required=True,
        help="Authoritative source URI for the Contents metadata.",
    )
    parser.add_argument(
        "--generated-at",
        default=dt.datetime.now(dt.timezone.utc)
        .replace(microsecond=0)
        .isoformat()
        .replace("+00:00", "Z"),
        help="Override generated-at timestamp (UTC ISO-8601).",
    )
    parser.add_argument(
        "--max-commands", type=int, default=75000, help="Maximum command keys."
    )
    parser.add_argument(
        "--max-packages-per-command",
        type=int,
        default=16,
        help="Maximum packages stored per command key.",
    )
    parser.add_argument(
        "--allowed-prefix",
        action="append",
        default=None,
        help=(
            "Allowed path prefix. Repeat for multiple. "
            "Defaults to /usr/bin, /bin, /usr/sbin, /sbin."
        ),
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()

    if args.max_commands <= 0:
        raise SystemExit("--max-commands must be > 0")
    if args.max_packages_per_command <= 0:
        raise SystemExit("--max-packages-per-command must be > 0")

    contents_files = [Path(path).resolve() for path in args.contents]
    for contents_file in contents_files:
        if not contents_file.exists():
            raise SystemExit(f"Contents file does not exist: {contents_file}")

    allowed_prefixes = tuple(args.allowed_prefix or DEFAULT_PREFIXES)
    commands = build_index(
        contents_files,
        allowed_prefixes=allowed_prefixes,
        max_commands=args.max_commands,
        max_packages_per_command=args.max_packages_per_command,
    )

    rendered = render_index(
        commands=commands,
        contents_files=contents_files,
        suite=args.suite,
        snapshot=args.snapshot,
        source_uri=args.source_uri,
        generated_at=args.generated_at,
        max_commands=args.max_commands,
        max_packages_per_command=args.max_packages_per_command,
    )

    output_path = Path(args.output)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(
        json.dumps(rendered, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
