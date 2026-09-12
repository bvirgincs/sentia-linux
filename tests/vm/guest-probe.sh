#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
set -eu

python3 - <<'PY'
import json
import os
import platform

memory_kib = None
with open("/proc/meminfo", encoding="utf-8") as handle:
    for line in handle:
        if line.startswith("MemTotal:"):
            memory_kib = int(line.split()[1])
            break

print(json.dumps({
    "schema_version": 1,
    "architecture": platform.machine(),
    "kernel": platform.release(),
    "memory_kib": memory_kib,
    "uid": os.getuid(),
    "ssh_authenticated": True,
}, sort_keys=True))
PY

