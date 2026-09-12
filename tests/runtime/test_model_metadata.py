import json
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
PROVENANCE = ROOT / "packaging/granite-model/provenance/granite-4.2-3b-q4_k_m.provenance.json"
VERIFY_SCRIPT = ROOT / "build/runtime/verify_granite_model.py"


class ModelMetadataTests(unittest.TestCase):
    def test_provenance_exact_pin(self) -> None:
        data = json.loads(PROVENANCE.read_text(encoding="utf-8"))
        model = data["model"]
        self.assertEqual(model["repository"], "ibm-granite/granite-4.2-3b-GGUF")
        self.assertEqual(model["revision"], "c40945d71cd90f249a56985e8155551a9188dc30")
        self.assertEqual(model["file"], "granite-4.2-3b-Q4_K_M.gguf")
        self.assertEqual(model["bytes"], 2244011552)
        self.assertEqual(
            model["sha256"],
            "e0406663965846ae22a403456eb826ccce5f450840491f71952f18a7cb78e7d5",
        )

    def test_verify_script_constants(self) -> None:
        text = VERIFY_SCRIPT.read_text(encoding="utf-8")
        self.assertIn("EXPECTED_REVISION = \"c40945d71cd90f249a56985e8155551a9188dc30\"", text)
        self.assertIn("EXPECTED_BYTES = 2244011552", text)
        self.assertIn(
            "EXPECTED_SHA256 = \"e0406663965846ae22a403456eb826ccce5f450840491f71952f18a7cb78e7d5\"",
            text,
        )
        self.assertIn("cryptographic_verification_performed\": False", text)


if __name__ == "__main__":
    unittest.main()
