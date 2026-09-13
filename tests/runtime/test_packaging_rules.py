import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
LLAMA_RULES = ROOT / "packaging/llama-cpp/debian/rules"
LLAMA_CONTROL = ROOT / "packaging/llama-cpp/debian/control"
GRANITE_RULES = ROOT / "packaging/granite-model/debian/rules"


class PackagingRulesTests(unittest.TestCase):
    def test_llama_build_flags(self) -> None:
        text = LLAMA_RULES.read_text(encoding="utf-8")
        for flag in [
            "-DGGML_BACKEND_DL=ON",
            "-DGGML_CPU_ALL_VARIANTS=ON",
            "-DGGML_NATIVE=OFF",
            "-DLLAMA_BUILD_SERVER=ON",
            "-DLLAMA_BUILD_UI=OFF",
            "-DLLAMA_USE_PREBUILT_UI=OFF",
            "-DLLAMA_SUBPROCESS=OFF",
            # Build parallelism is bounded and caller-selectable; an unbounded
            # -j on the build host exhausts its memory during the ggml compile.
            "-j$(LLAMA_JOBS)",
            "LLAMA_JOBS ?=",
        ]:
            self.assertIn(flag, text)

    def test_llama_build_depends_declared(self) -> None:
        text = LLAMA_CONTROL.read_text(encoding="utf-8")
        for dep in ["cmake", "ninja-build", "g++", "pkgconf", "python3"]:
            self.assertIn(dep, text)

    def test_granite_rules_use_verifier(self) -> None:
        text = GRANITE_RULES.read_text(encoding="utf-8")
        self.assertIn("verify_granite_model.py", text)
        self.assertIn("granite-4.2-3b-Q4_K_M.gguf", text)


if __name__ == "__main__":
    unittest.main()
