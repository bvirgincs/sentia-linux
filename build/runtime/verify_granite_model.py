#!/usr/bin/env python3
"""Verify Sentia-pinned Granite model metadata and checksums.

This performs local integrity checks and signature bundle structure checks.
It intentionally does NOT claim cryptographic signature verification success,
which requires the official IBM model-signing verification flow.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import re
import sys
from pathlib import Path

EXPECTED_REPO = "ibm-granite/granite-4.2-3b-GGUF"
EXPECTED_REVISION = "c40945d71cd90f249a56985e8155551a9188dc30"
EXPECTED_MODEL_FILE = "granite-4.2-3b-Q4_K_M.gguf"
EXPECTED_BYTES = 2244011552
EXPECTED_SHA256 = "e0406663965846ae22a403456eb826ccce5f450840491f71952f18a7cb78e7d5"

IBM_SIGNATURE_DOC = "https://www.ibm.com/granite/docs/model-standards/signature-verification"
SIGSTORE_VERIFY_DOC = "https://docs.sigstore.dev/cosign/verifying/verify/"


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def extract_ibm_emails(certificate_der_b64: str) -> list[str]:
    cert_bytes = base64.b64decode(certificate_der_b64)
    text = cert_bytes.decode("latin-1", errors="ignore")
    found = sorted(set(re.findall(r"[A-Za-z0-9._%+-]+@ibm\.com", text)))
    return found


def load_sig(sig_path: Path) -> dict:
    with sig_path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def decode_dsse_payload(sig_data: dict) -> dict:
    payload_b64 = sig_data.get("dsseEnvelope", {}).get("payload", "")
    if not payload_b64:
        return {}
    payload = base64.b64decode(payload_b64)
    return json.loads(payload.decode("utf-8"))


def find_resource_digest(payload: dict, name: str) -> str | None:
    resources = payload.get("predicate", {}).get("resources", [])
    for resource in resources:
        if resource.get("name") == name:
            return resource.get("digest")
    return None


def tree_entry(tree_data: list[dict], path_name: str) -> dict | None:
    for entry in tree_data:
        if entry.get("path") == path_name:
            return entry
    return None


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", required=True)
    parser.add_argument("--sig", required=True)
    parser.add_argument("--readme", required=True)
    parser.add_argument("--tree-json", default="")
    parser.add_argument("--report", required=True)
    parser.add_argument("--allow-missing-model", action="store_true")
    args = parser.parse_args()

    model_path = Path(args.model)
    sig_path = Path(args.sig)
    readme_path = Path(args.readme)
    report_path = Path(args.report)
    report_path.parent.mkdir(parents=True, exist_ok=True)

    report: dict[str, object] = {
        "expected": {
            "repo": EXPECTED_REPO,
            "revision": EXPECTED_REVISION,
            "file": EXPECTED_MODEL_FILE,
            "bytes": EXPECTED_BYTES,
            "sha256": EXPECTED_SHA256,
        },
        "inputs": {
            "model": str(model_path),
            "sig": str(sig_path),
            "readme": str(readme_path),
            "tree_json": args.tree_json,
        },
        "checks": {},
        "signature": {
            "cryptographic_verification_performed": False,
            "cryptographic_verification_status": "not_performed",
            "official_procedure": {
                "ibm_doc": IBM_SIGNATURE_DOC,
                "sigstore_doc": SIGSTORE_VERIFY_DOC,
            },
            "note": "model.sig structure was inspected; full signature validation requires official toolchain and identity policy.",
        },
    }

    checks = report["checks"]

    checks["readme_exists"] = readme_path.is_file()
    checks["sig_exists"] = sig_path.is_file()

    model_exists = model_path.is_file()
    checks["model_exists"] = model_exists

    model_ok = True
    if model_exists:
        actual_size = model_path.stat().st_size
        actual_sha = sha256_file(model_path)
        checks["model_size_bytes"] = actual_size
        checks["model_sha256"] = actual_sha
        checks["model_size_matches"] = actual_size == EXPECTED_BYTES
        checks["model_sha_matches"] = actual_sha == EXPECTED_SHA256
        model_ok = bool(checks["model_size_matches"] and checks["model_sha_matches"])
    elif not args.allow_missing_model:
        model_ok = False

    sig_ok = False
    if sig_path.is_file():
        sig_data = load_sig(sig_path)
        checks["sig_media_type"] = sig_data.get("mediaType")
        payload = decode_dsse_payload(sig_data)
        digest = find_resource_digest(payload, EXPECTED_MODEL_FILE)
        checks["sig_payload_digest_for_model"] = digest
        checks["sig_payload_digest_matches"] = digest == EXPECTED_SHA256
        certificate_raw = (
            sig_data.get("verificationMaterial", {})
            .get("certificate", {})
            .get("rawBytes", "")
        )
        checks["sig_certificate_emails"] = (
            extract_ibm_emails(certificate_raw) if certificate_raw else []
        )
        checks["sig_has_tlog_entries"] = bool(
            sig_data.get("verificationMaterial", {}).get("tlogEntries")
        )
        sig_ok = bool(
            checks["sig_media_type"]
            == "application/vnd.dev.sigstore.bundle.v0.3+json"
            and checks["sig_payload_digest_matches"]
        )

    tree_ok = True
    if args.tree_json:
        tree_path = Path(args.tree_json)
        checks["tree_json_exists"] = tree_path.is_file()
        if tree_path.is_file():
            tree_data = json.loads(tree_path.read_text(encoding="utf-8"))
            model_entry = tree_entry(tree_data, EXPECTED_MODEL_FILE)
            sig_entry = tree_entry(tree_data, "model.sig")
            checks["tree_model_entry"] = model_entry
            checks["tree_sig_entry"] = sig_entry
            tree_ok = bool(
                model_entry
                and model_entry.get("size") == EXPECTED_BYTES
                and model_entry.get("lfs", {}).get("oid") == EXPECTED_SHA256
            )
        else:
            tree_ok = False

    success = model_ok and sig_ok and tree_ok and bool(checks["readme_exists"])
    report["success"] = success

    report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")

    if not success:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
