#!/usr/bin/env python3
"""Inspect pinned llama.cpp source flags from upstream commit.

This script validates that required CMake options and runtime CLI flags exist in
llama.cpp v0.4.0 commit 5266f24da75dc449bd56cbed7addb9c8e4a6a73e.
"""

from __future__ import annotations

import argparse
import json
import sys
import urllib.request
from pathlib import Path

OWNER = "ggml-org"
REPO = "llama.cpp"
TAG = "v0.4.0"
COMMIT = "5266f24da75dc449bd56cbed7addb9c8e4a6a73e"

ROOT_CMAKE_URL = (
    f"https://raw.githubusercontent.com/{OWNER}/{REPO}/{COMMIT}/CMakeLists.txt"
)
GGML_CMAKE_URL = (
    f"https://raw.githubusercontent.com/{OWNER}/{REPO}/{COMMIT}/ggml/CMakeLists.txt"
)
ARG_CPP_URL = (
    f"https://raw.githubusercontent.com/{OWNER}/{REPO}/{COMMIT}/common/arg.cpp"
)
TAG_REF_API = f"https://api.github.com/repos/{OWNER}/{REPO}/git/ref/tags/{TAG}"

REQUIRED_GGML_TOKENS = [
    'option(GGML_BACKEND_DL',
    'option(GGML_CPU_ALL_VARIANTS',
    'option(GGML_NATIVE',
]

REQUIRED_ROOT_TOKENS = [
    'option(LLAMA_BUILD_SERVER',
    'option(LLAMA_BUILD_UI',
    'option(LLAMA_USE_PREBUILT_UI',
    'option(LLAMA_SUBPROCESS',
]

REQUIRED_ARG_TOKENS = [
    '{"--host"}',
    'bind to an UNIX socket if the address ends with .sock',
    '{"--no-ui", "--no-webui"}',
    '{"--tools"}',
    '{"--mcp-servers-config"}',
    '{"--sleep-idle-seconds"}',
    '{"--metrics"}',
]


def fetch_text(url: str) -> str:
    with urllib.request.urlopen(url, timeout=60) as response:
        return response.read().decode("utf-8")


def fetch_json(url: str) -> dict:
    with urllib.request.urlopen(url, timeout=60) as response:
        return json.loads(response.read().decode("utf-8"))


def line_numbers_with_token(text: str, token: str) -> list[int]:
    lines = text.splitlines()
    return [idx + 1 for idx, line in enumerate(lines) if token in line]


def resolve_tag_commit() -> str:
    ref = fetch_json(TAG_REF_API)
    obj = ref.get("object", {})
    obj_type = obj.get("type")
    obj_sha = obj.get("sha", "")
    if obj_type == "commit":
        return obj_sha
    if obj_type == "tag":
        tag_obj = fetch_json(
            f"https://api.github.com/repos/{OWNER}/{REPO}/git/tags/{obj_sha}"
        )
        return tag_obj.get("object", {}).get("sha", "")
    return ""


def validate(required_tokens: list[str], text: str, section: str) -> dict:
    missing = []
    present = {}
    for token in required_tokens:
        lines = line_numbers_with_token(text, token)
        if not lines:
            missing.append(token)
        else:
            present[token] = lines
    return {"section": section, "missing": missing, "present": present}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--output",
        default="/home/ubuntu/sentia-linux/.build/runtime/logs/llama-source-inspection.json",
        help="Path to write inspection report JSON",
    )
    args = parser.parse_args()

    output_path = Path(args.output)
    output_path.parent.mkdir(parents=True, exist_ok=True)

    root_cmake = fetch_text(ROOT_CMAKE_URL)
    ggml_cmake = fetch_text(GGML_CMAKE_URL)
    arg_cpp = fetch_text(ARG_CPP_URL)
    resolved_tag_commit = resolve_tag_commit()

    checks = [
        validate(REQUIRED_ROOT_TOKENS, root_cmake, "root_cmake"),
        validate(REQUIRED_GGML_TOKENS, ggml_cmake, "ggml_cmake"),
        validate(REQUIRED_ARG_TOKENS, arg_cpp, "arg_cpp"),
    ]

    missing_tokens = {
        check["section"]: check["missing"] for check in checks if check["missing"]
    }
    success = not missing_tokens and resolved_tag_commit.startswith(COMMIT)

    report = {
        "owner": OWNER,
        "repo": REPO,
        "tag": TAG,
        "expected_commit": COMMIT,
        "resolved_tag_commit": resolved_tag_commit,
        "urls": {
            "root_cmake": ROOT_CMAKE_URL,
            "ggml_cmake": GGML_CMAKE_URL,
            "arg_cpp": ARG_CPP_URL,
            "tag_ref": TAG_REF_API,
        },
        "checks": checks,
        "missing_tokens": missing_tokens,
        "success": success,
        "notes": [
            "GGML_BACKEND_DL + GGML_CPU_ALL_VARIANTS + GGML_NATIVE validation is sourced from upstream CMake.",
            "Runtime hardening flags (--no-webui, --tools, --mcp-servers-config, --sleep-idle-seconds, Unix socket host) are sourced from common/arg.cpp.",
        ],
    }

    output_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")

    if not success:
        print(json.dumps(report, indent=2))
        return 1

    print(f"wrote inspection report: {output_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
