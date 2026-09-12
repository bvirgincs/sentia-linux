from __future__ import annotations

import importlib.util
import json
import os
import shutil
import subprocess
import sys
import uuid
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "build" / "release" / "release_artifacts.py"


def _load_module():
    spec = importlib.util.spec_from_file_location("release_artifacts", SCRIPT)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load module from {SCRIPT}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


MODULE = _load_module()


class ReleaseArtifactCLITests(unittest.TestCase):
    def setUp(self) -> None:
        self._workspaces: list[Path] = []

    def tearDown(self) -> None:
        for path in reversed(self._workspaces):
            shutil.rmtree(path, ignore_errors=True)

    def workspace(self, name: str) -> Path:
        path = ROOT / "tests" / "release" / f".scratch_{name}_{uuid.uuid4().hex}"
        path.mkdir()
        self._workspaces.append(path)
        return path

    def run_cli(self, *args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
        result = subprocess.run(
            [sys.executable, str(SCRIPT), *args],
            cwd=ROOT,
            capture_output=True,
            text=True,
        )
        if check and result.returncode != 0:
            self.fail(
                f"command failed: {args}\nstdout:\n{result.stdout}\nstderr:\n{result.stderr}"
            )
        if not check and result.returncode == 0:
            self.fail(f"command unexpectedly succeeded: {args}\nstdout:\n{result.stdout}")
        return result

    def make_fixture(self, chunk_size: int = 11):
        work = self.workspace("fixture")
        source = work / "bundle.iso"
        payload = (b"Sentia release artifact split test payload\n" * 5) + b"!"
        source.write_bytes(payload)
        parts_dir = work / "parts"
        parts_dir.mkdir()
        result = self.run_cli(
            "split",
            str(source),
            "--output-dir",
            str(parts_dir),
            "--chunk-size",
            str(chunk_size),
        )
        manifest = MODULE.manifest_path_for(parts_dir, source.name)
        self.assertTrue(manifest.exists(), result.stdout)
        manifest_data = json.loads(manifest.read_text(encoding="utf-8"))
        return work, source, payload, parts_dir, manifest, manifest_data

    def rewrite_manifest(self, manifest: Path, data: dict) -> None:
        manifest.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")

    def test_help_mentions_subcommands(self) -> None:
        result = self.run_cli("--help")
        self.assertEqual(result.returncode, 0)
        self.assertIn("split", result.stdout)
        self.assertIn("verify", result.stdout)
        self.assertIn("join", result.stdout)

    def test_roundtrip_split_verify_join(self) -> None:
        _, source, payload, parts_dir, manifest, manifest_data = self.make_fixture(chunk_size=17)

        self.assertEqual(manifest_data["manifest_version"], 1)
        self.assertEqual(manifest_data["chunk_size"], 17)
        self.assertEqual(manifest_data["original"]["basename"], source.name)
        self.assertEqual(manifest_data["original"]["size"], len(payload))
        self.assertEqual(manifest_data["original"]["sha256"], MODULE.hashlib.sha256(payload).hexdigest())
        self.assertGreaterEqual(len(manifest_data["parts"]), 3)

        expected_names = [
            MODULE.part_name_for(source.name, index) for index in range(1, len(manifest_data["parts"]) + 1)
        ]
        self.assertEqual([part["name"] for part in manifest_data["parts"]], expected_names)

        verify = self.run_cli("verify", str(manifest), "--parts-dir", str(parts_dir))
        self.assertIn("verified", verify.stdout.lower())

        output = parts_dir / source.name
        join = self.run_cli(
            "join",
            str(manifest),
            "--parts-dir",
            str(parts_dir),
            "--output",
            str(output),
        )
        self.assertIn("joined", join.stdout.lower())
        self.assertEqual(output.read_bytes(), payload)

    def test_corruption_detected(self) -> None:
        _, source, payload, parts_dir, manifest, manifest_data = self.make_fixture(chunk_size=13)
        first_part = parts_dir / manifest_data["parts"][0]["name"]
        bytes_data = bytearray(first_part.read_bytes())
        bytes_data[0] ^= 0xFF
        first_part.write_bytes(bytes(bytes_data))
        result = self.run_cli("verify", str(manifest), "--parts-dir", str(parts_dir), check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("sha256 mismatch", result.stderr.lower())

    def test_traversal_and_absolute_paths_rejected(self) -> None:
        _, _, _, parts_dir, manifest, manifest_data = self.make_fixture(chunk_size=19)

        manifest_data["parts"][0]["name"] = "../evil"
        self.rewrite_manifest(manifest, manifest_data)
        traversal = self.run_cli("verify", str(manifest), "--parts-dir", str(parts_dir), check=False)
        self.assertNotEqual(traversal.returncode, 0)
        self.assertIn("path", traversal.stderr.lower())

        manifest_data = json.loads(manifest.read_text(encoding="utf-8"))
        manifest_data["parts"][0]["name"] = "/etc/passwd"
        self.rewrite_manifest(manifest, manifest_data)
        absolute = self.run_cli("verify", str(manifest), "--parts-dir", str(parts_dir), check=False)
        self.assertNotEqual(absolute.returncode, 0)
        self.assertIn("absolute", absolute.stderr.lower())

    def test_symlink_part_rejected(self) -> None:
        _, source, _, parts_dir, manifest, manifest_data = self.make_fixture(chunk_size=15)
        first_part = parts_dir / manifest_data["parts"][0]["name"]
        first_part.unlink()
        os.symlink(source, first_part)
        result = self.run_cli("verify", str(manifest), "--parts-dir", str(parts_dir), check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("symlink", result.stderr.lower())

    def test_duplicate_names_and_gaps_rejected(self) -> None:
        _, _, _, parts_dir, manifest, manifest_data = self.make_fixture(chunk_size=11)

        manifest_data["parts"][1]["name"] = manifest_data["parts"][0]["name"]
        self.rewrite_manifest(manifest, manifest_data)
        duplicate = self.run_cli("verify", str(manifest), "--parts-dir", str(parts_dir), check=False)
        self.assertNotEqual(duplicate.returncode, 0)
        self.assertIn("duplicate", duplicate.stderr.lower())

        manifest_data = json.loads(manifest.read_text(encoding="utf-8"))
        manifest_data["parts"][1]["name"] = MODULE.part_name_for(manifest_data["original"]["basename"], 99)
        self.rewrite_manifest(manifest, manifest_data)
        gap = self.run_cli("verify", str(manifest), "--parts-dir", str(parts_dir), check=False)
        self.assertNotEqual(gap.returncode, 0)
        self.assertIn("gap", gap.stderr.lower())

    def test_schema_bool_int_rejected(self) -> None:
        _, _, _, parts_dir, manifest, manifest_data = self.make_fixture(chunk_size=23)
        manifest_data["chunk_size"] = True
        self.rewrite_manifest(manifest, manifest_data)
        result = self.run_cli("verify", str(manifest), "--parts-dir", str(parts_dir), check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("integer", result.stderr.lower())

    def test_overwrite_rejected(self) -> None:
        work, source, _, parts_dir, manifest, manifest_data = self.make_fixture(chunk_size=17)
        first_part = parts_dir / MODULE.part_name_for(source.name, 1)
        sentinel = b"do not clobber"
        first_part.write_bytes(sentinel)

        split = self.run_cli(
            "split",
            str(source),
            "--output-dir",
            str(parts_dir),
            "--chunk-size",
            "17",
            check=False,
        )
        self.assertNotEqual(split.returncode, 0)
        self.assertEqual(first_part.read_bytes(), sentinel)

        output = parts_dir / source.name
        output.write_bytes(sentinel)
        join = self.run_cli(
            "join",
            str(manifest),
            "--parts-dir",
            str(parts_dir),
            "--output",
            str(output),
            check=False,
        )
        self.assertNotEqual(join.returncode, 0)
        self.assertEqual(output.read_bytes(), sentinel)

    def test_unreasonable_chunk_size_rejected(self) -> None:
        work = self.workspace("limit")
        source = work / "bundle.deb"
        source.write_bytes(b"small fixture data")
        parts_dir = work / "parts"
        parts_dir.mkdir()
        result = self.run_cli(
            "split",
            str(source),
            "--output-dir",
            str(parts_dir),
            "--chunk-size",
            str(MODULE.MAX_PART_SIZE + 1),
            check=False,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("chunk size", result.stderr.lower())


if __name__ == "__main__":
    unittest.main()
