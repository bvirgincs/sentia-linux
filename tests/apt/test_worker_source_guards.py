import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
WORKER_CPP = ROOT / "native" / "sentia-apt" / "src" / "apt_worker.cpp"


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


if __name__ == "__main__":
    unittest.main()
