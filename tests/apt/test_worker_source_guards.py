import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
WORKER_CPP = ROOT / "native" / "sentia-apt" / "src" / "apt_worker.cpp"
MAIN_CPP = ROOT / "native" / "sentia-apt" / "src" / "main.cpp"
CMAKE_FILE = ROOT / "native" / "sentia-apt" / "CMakeLists.txt"


class WorkerSourceGuardTests(unittest.TestCase):
    def _handle_execute_block(self) -> str:
        source = WORKER_CPP.read_text(encoding="utf-8")
        start = source.find("json HandleExecute(")
        self.assertNotEqual(start, -1)
        end = source.find("json HandleMutatingPlanExecute(", start)
        self.assertNotEqual(end, -1)
        return source[start:end]

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

    def test_install_compat_symlink_path(self) -> None:
        source = CMAKE_FILE.read_text(encoding="utf-8")
        self.assertIn("create_symlink", source)
        self.assertIn("sentia-apt-worker", source)
        self.assertIn("/sentia-apt", source)

    def test_supports_shared_tool_request_fields(self) -> None:
        source = WORKER_CPP.read_text(encoding="utf-8")
        self.assertIn('request.contains("name")', source)
        self.assertIn('request.contains("args")', source)
        self.assertIn('"sentia.v1"', source)

    def test_execute_re_resolves_under_native_locks(self) -> None:
        execute_block = self._handle_execute_block()
        self.assertIn(
            "BuildPlannedTransaction(request, true, require_packages, operation)",
            execute_block,
        )
        self.assertIn("cache_file.Open(nullptr, true)", execute_block)
        self.assertIn(
            "BuildCanonicalPlan(operation, packages, *cache, dep_cache, true)",
            execute_block,
        )

    def test_execute_compares_digest_before_apply(self) -> None:
        execute_block = self._handle_execute_block()
        pre_lock_compare = execute_block.find(
            "if (planned.digest != approval.plan_digest)"
        )
        lock_open = execute_block.find("cache_file.Open(nullptr, true)")
        re_resolve_compare = execute_block.find(
            "if (execute_plan.digest != approval.plan_digest)"
        )
        do_install = execute_block.find("ExecuteResolvedPlan(cache_file, dep_cache)")

        self.assertGreaterEqual(pre_lock_compare, 0)
        self.assertGreaterEqual(lock_open, 0)
        self.assertGreaterEqual(re_resolve_compare, 0)
        self.assertGreaterEqual(do_install, 0)
        self.assertLess(pre_lock_compare, lock_open)
        self.assertLess(re_resolve_compare, do_install)


if __name__ == "__main__":
    unittest.main()
