import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
WORKER_CPP = ROOT / "native" / "sentia-apt" / "src" / "apt_worker.cpp"
MAIN_CPP = ROOT / "native" / "sentia-apt" / "src" / "main.cpp"


class WorkerSourceGuardTests(unittest.TestCase):
    def test_no_shell_execution_path(self) -> None:
        source = WORKER_CPP.read_text(encoding="utf-8")
        self.assertNotIn("apt-get -s", source)
        self.assertNotIn("system(", source)
        self.assertNotIn("popen(", source)
        self.assertNotIn("sh -c", source)

    def test_uses_libapt_packagemanager_execution(self) -> None:
        source = WORKER_CPP.read_text(encoding="utf-8")
        self.assertIn("pkgPackageManager", source)
        self.assertIn("DoInstall", source)

    def test_canonical_plan_excludes_lock_mode(self) -> None:
        source = WORKER_CPP.read_text(encoding="utf-8")
        start = source.find("planned.canonical_plan =")
        self.assertNotEqual(start, -1)
        end = source.find("planned.digest =", start)
        self.assertNotEqual(end, -1)
        canonical_block = source[start:end]
        self.assertNotIn('"with_lock"', canonical_block)

    def test_entrypoint_bounds_stdin(self) -> None:
        source = MAIN_CPP.read_text(encoding="utf-8")
        self.assertIn("kMaxRequestBytes", source)
        self.assertIn("request_too_large", source)
        self.assertIn("ReadBoundedStdin", source)


if __name__ == "__main__":
    unittest.main()
