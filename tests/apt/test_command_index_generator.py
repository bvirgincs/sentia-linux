import gzip
import json
import shutil
import subprocess
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
GENERATOR = ROOT / "build" / "command-index" / "generate_command_index.py"
ARTIFACT_ROOT = Path(__file__).resolve().parent / "_artifacts"


class CommandIndexGeneratorTests(unittest.TestCase):
    def setUp(self) -> None:
        self.work = ARTIFACT_ROOT / self._testMethodName
        if self.work.exists():
            shutil.rmtree(self.work)
        self.work.mkdir(parents=True, exist_ok=True)

    def tearDown(self) -> None:
        if self.work.exists():
            shutil.rmtree(self.work)

    def _run_generator(self, contents_lines: str, *extra_args: str) -> dict:
        contents_path = self.work / "Contents-amd64.sample.gz"
        with gzip.open(contents_path, "wt", encoding="utf-8") as handle:
            handle.write(contents_lines)

        output_path = self.work / "command-index.json"
        command = [
            sys.executable,
            str(GENERATOR),
            "--contents",
            str(contents_path),
            "--output",
            str(output_path),
            "--suite",
            "trixie",
            "--snapshot",
            "2026-09-12T00:00:00Z",
            "--source-uri",
            "https://snapshot.debian.org/archive/debian/20260912T000000Z/",
            "--generated-at",
            "2026-09-12T00:00:00Z",
            *extra_args,
        ]
        subprocess.run(command, cwd=ROOT, check=True)
        return json.loads(output_path.read_text(encoding="utf-8"))

    def test_generates_command_lookup_from_contents(self) -> None:
        rendered = self._run_generator(
            "\n".join(
                [
                    "usr/bin/python3 interpreters/python3-minimal",
                    "usr/bin/apt admin/apt",
                    "usr/sbin/adduser admin/adduser,oldlibs/adduser-legacy",
                    "usr/share/doc/ignored doc/ignored",
                ]
            )
            + "\n"
        )

        self.assertIn("python3", rendered["commands"])
        self.assertEqual(
            rendered["commands"]["python3"]["packages"], ["python3-minimal"]
        )
        self.assertEqual(
            rendered["commands"]["adduser"]["packages"],
            ["adduser", "adduser-legacy"],
        )
        self.assertNotIn("ignored", rendered["commands"])

    def test_respects_command_cap(self) -> None:
        rendered = self._run_generator(
            "\n".join(
                [
                    "usr/bin/a foo/pkg-a",
                    "usr/bin/b foo/pkg-b",
                    "usr/bin/c foo/pkg-c",
                ]
            )
            + "\n",
            "--max-commands",
            "2",
        )
        self.assertEqual(len(rendered["commands"]), 2)

    def test_provenance_includes_input_hash(self) -> None:
        rendered = self._run_generator("usr/bin/ls utils/coreutils\n")
        provenance_files = rendered["provenance"]["contents_files"]
        self.assertEqual(len(provenance_files), 1)
        self.assertRegex(provenance_files[0]["sha256"], r"^[a-f0-9]{64}$")


if __name__ == "__main__":
    unittest.main()
