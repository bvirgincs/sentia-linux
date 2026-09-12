import os
import unittest
from pathlib import Path


class IsolatedTransactionHarnessTests(unittest.TestCase):
    @unittest.skipUnless(
        os.getenv("SENTIA_APT_TEST_CHROOT"),
        "Set SENTIA_APT_TEST_CHROOT to run mutation tests in isolated artifacts.",
    )
    def test_chroot_path_isolated_guardrails(self) -> None:
        chroot_path = Path(os.environ["SENTIA_APT_TEST_CHROOT"]).resolve()
        self.assertTrue(chroot_path.exists(), "chroot path must exist")
        self.assertNotEqual(chroot_path, Path("/"), "chroot path must not be /")
        self.assertNotIn(
            str(Path.home()),
            str(chroot_path),
            "do not run mutation tests in host home paths",
        )


if __name__ == "__main__":
    unittest.main()
