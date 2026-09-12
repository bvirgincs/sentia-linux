#!/usr/bin/env python3
from __future__ import annotations

import datetime as dt
import os
from pathlib import Path
import re
import shutil
import sys


def fail(message: str) -> None:
    print(f"error: {message}", file=sys.stderr)
    sys.exit(1)


def normalize_os_release_lines(raw: str) -> list[str]:
    normalized = re.sub(r"\s+(?=[A-Z_]+=)", "\n", raw.strip())
    return [line.strip() for line in normalized.splitlines() if line.strip()]


def rewrite_os_release(path: Path) -> None:
    raw = path.read_text(encoding="utf-8")
    lines = normalize_os_release_lines(raw)

    values: dict[str, str] = {}
    order: list[str] = []
    for line in lines:
        if "=" not in line:
            continue
        key, value = line.split("=", 1)
        key = key.strip()
        values[key] = value.strip()
        if key not in order:
            order.append(key)

    values["PRETTY_NAME"] = '"Sentia Linux 0.1 (Debian #OSNAME# 13 (trixie))"'
    values["NAME"] = '"Sentia Linux"'
    values["ID"] = "sentia"
    values["ID_LIKE"] = "debian"

    for key in ("PRETTY_NAME", "NAME", "ID"):
        if key not in order:
            order.append(key)
    if "ID_LIKE" not in order:
        if "ID" in order:
            order.insert(order.index("ID") + 1, "ID_LIKE")
        else:
            order.append("ID_LIKE")

    rendered = "\n".join(f"{key}={values[key]}" for key in order) + "\n"
    path.write_text(rendered, encoding="utf-8")


def rewrite_rules(path: Path) -> None:
    text = path.read_text(encoding="utf-8")
    if "VENDORFILE = sentia" not in text:
        if "VENDORFILE = debian" not in text:
            fail("could not find 'VENDORFILE = debian' in debian/rules")
        text = text.replace("VENDORFILE = debian", "VENDORFILE = sentia", 1)

    required_lines = (
        "mv $(DESTDIR)/etc/os-release $(DESTDIR)/usr/lib/os-release",
        "ln -s ../usr/lib/os-release $(DESTDIR)/etc/os-release",
    )
    for line in required_lines:
        if line not in text:
            fail(f"debian/rules missing required os-release ownership rule: {line}")

    path.write_text(text, encoding="utf-8")


def rewrite_postinst(path: Path) -> None:
    text = path.read_text(encoding="utf-8")

    if "current_vendor=\"$(readlink \"$DPKG_ROOT/etc/dpkg/origins/default\")\"" in text:
        return

    old_block = re.compile(
        r"""if \[ ! -e "\$DPKG_ROOT/etc/dpkg/origins/default" \]; then
\s+if \[ -e "\$DPKG_ROOT/etc/dpkg/origins/#VENDORFILE#" \]; then
\s+ln -sf #VENDORFILE# "\$DPKG_ROOT/etc/dpkg/origins/default"
\s+fi
fi
""",
        re.MULTILINE,
    )
    new_block = """if [ -L "$DPKG_ROOT/etc/dpkg/origins/default" ]; then
  current_vendor="$(readlink "$DPKG_ROOT/etc/dpkg/origins/default")"
  if [ "$current_vendor" = "debian" ] && [ -e "$DPKG_ROOT/etc/dpkg/origins/#VENDORFILE#" ]; then
    ln -sf #VENDORFILE# "$DPKG_ROOT/etc/dpkg/origins/default"
  fi
elif [ ! -e "$DPKG_ROOT/etc/dpkg/origins/default" ]; then
  if [ -e "$DPKG_ROOT/etc/dpkg/origins/#VENDORFILE#" ]; then
    ln -sf #VENDORFILE# "$DPKG_ROOT/etc/dpkg/origins/default"
  fi
fi
"""

    text, replacements = old_block.subn(new_block, text, count=1)
    if replacements != 1:
        fail("could not find expected vendor-default block in debian/postinst")
    path.write_text(text, encoding="utf-8")


def prepend_changelog_entry(path: Path) -> None:
    text = path.read_text(encoding="utf-8")
    first_line = text.splitlines()[0]
    match = re.match(r"^(base-files) \(([^)]+)\) (\S+); urgency=(\S+)$", first_line)
    if not match:
        fail("debian/changelog first line is not in expected format")

    current_version = match.group(2)
    if "+sentia" in current_version:
        return

    new_version = f"{current_version}+sentia1"
    source_date_epoch = os.environ.get("SOURCE_DATE_EPOCH")
    if source_date_epoch:
        now = dt.datetime.fromtimestamp(int(source_date_epoch), tz=dt.timezone.utc)
    else:
        now = dt.datetime.now(tz=dt.timezone.utc)

    stamp = now.strftime("%a, %d %b %Y %H:%M:%S +0000")
    new_entry = (
        f"base-files ({new_version}) trixie; urgency=medium\n\n"
        "  * Set Sentia identity in os-release while retaining Debian base metadata.\n"
        "  * Add dpkg Sentia origin and migrate default origin only when still Debian.\n\n"
        f" -- Sentia Packaging Team <packaging@sentia.invalid>  {stamp}\n\n"
    )
    path.write_text(new_entry + text, encoding="utf-8")


def main() -> None:
    if len(sys.argv) != 2:
        fail("usage: apply-sentia-delta.py <unpacked-base-files-source>")

    source_tree = Path(sys.argv[1]).resolve()
    if not source_tree.is_dir():
        fail(f"source tree not found: {source_tree}")

    required = [
        source_tree / "etc" / "os-release",
        source_tree / "debian" / "rules",
        source_tree / "debian" / "postinst",
        source_tree / "debian" / "changelog",
        source_tree / "origins" / "debian",
    ]
    for path in required:
        if not path.exists():
            fail(f"required upstream file missing: {path}")

    rewrite_os_release(source_tree / "etc" / "os-release")
    rewrite_rules(source_tree / "debian" / "rules")
    rewrite_postinst(source_tree / "debian" / "postinst")
    prepend_changelog_entry(source_tree / "debian" / "changelog")

    template_origin = Path(__file__).resolve().parent / "origins" / "sentia"
    if not template_origin.exists():
        fail(f"missing sentia origin template: {template_origin}")
    shutil.copyfile(template_origin, source_tree / "origins" / "sentia")


if __name__ == "__main__":
    main()
