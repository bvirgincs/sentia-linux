#!/usr/bin/env python3
"""The declared builder dependency set must satisfy every package's build-deps.

A Sentia build that only works because somebody installed a -dev package on one
particular host by hand is not a reproducible build. This caught exactly that:
sentia-ai needed four toolkit -dev packages that the manifest never listed, so
the build succeeded on a hand-modified runner and failed on a clean one.
"""
from __future__ import annotations

import re
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
MANIFEST = REPO_ROOT / "build" / "manifests" / "builder-dependencies.txt"

# Virtual packages and alternatives that a real package provides instead.
PROVIDED_BY = {
    "debhelper-compat": "debhelper",
    "pkgconf": "pkg-config",
}


def manifest_packages() -> set[str]:
    packages = set()
    for line in MANIFEST.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if line and not line.startswith("#"):
            packages.add(line)
    return packages


def build_depends(control: Path) -> set[str]:
    """Parse the Build-Depends field of a Debian control file."""
    text = control.read_text(encoding="utf-8")
    match = re.search(
        r"^Build-Depends:(.*?)(?=^[A-Z][A-Za-z0-9-]*:)", text, re.S | re.M
    )
    if match is None:
        return set()
    names = set()
    for item in match.group(1).split(","):
        item = item.strip()
        if not item:
            continue
        # Strip version constraints, architecture qualifiers and alternatives.
        name = re.split(r"[\s(\[|]", item, maxsplit=1)[0].strip()
        if name:
            names.add(name)
    return names


class BuilderDependencyTest(unittest.TestCase):
    def test_manifest_satisfies_every_package_build_depends(self) -> None:
        available = manifest_packages()
        missing: dict[str, set[str]] = {}
        for control in sorted((REPO_ROOT / "packaging").glob("*/debian/control")):
            required = build_depends(control)
            unmet = {
                name
                for name in required
                if name not in available
                and PROVIDED_BY.get(name) not in available
            }
            if unmet:
                missing[str(control.relative_to(REPO_ROOT))] = unmet
        self.assertEqual(
            missing,
            {},
            "builder-dependencies.txt does not satisfy these Build-Depends; a "
            "clean build host would fail:\n"
            + "\n".join(f"  {k}: {sorted(v)}" for k, v in missing.items()),
        )

    def test_manifest_is_sorted_and_unique(self) -> None:
        entries = [
            line.strip()
            for line in MANIFEST.read_text(encoding="utf-8").splitlines()
            if line.strip() and not line.startswith("#")
        ]
        self.assertEqual(entries, sorted(entries), "manifest must stay sorted")
        self.assertEqual(len(entries), len(set(entries)), "manifest has duplicates")


if __name__ == "__main__":
    unittest.main()
