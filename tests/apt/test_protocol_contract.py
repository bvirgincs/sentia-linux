import json
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
PROTOCOL_DIR = ROOT / "native" / "sentia-apt" / "protocol"


class ProtocolContractTests(unittest.TestCase):
    def test_request_schema_lists_required_operations(self) -> None:
        schema = json.loads((PROTOCOL_DIR / "request.schema.json").read_text())
        operations = set(schema["properties"]["operation"]["enum"])
        expected = {
            "apt_search",
            "apt_package_info",
            "apt_package_policy",
            "apt_simulate_install",
            "apt_install",
            "apt_remove",
            "apt_update",
            "apt_upgrade",
            "package_owns_file",
            "diagnostics",
        }
        self.assertTrue(expected.issubset(operations))

    def test_broker_contract_execute_requires_approval_fields(self) -> None:
        contract = json.loads((PROTOCOL_DIR / "broker-contract.json").read_text())
        self.assertEqual(contract["method_version"], 1)
        self.assertEqual(contract["protocol_version"], "1.0")
        self.assertEqual(contract["binary_path"], "/usr/libexec/sentia/sentia-apt")
        required = set(contract["required_approval_fields"])
        self.assertEqual(
            required,
            {
                "plan_digest",
                "broker_session_id",
                "authorization_id",
                "expires_at",
                "allow_source_change",
                "allow_essential_removal",
                "allow_held_change",
            },
        )
        execute_example = contract["examples"]["install_execute_request"]["approval"]
        self.assertRegex(execute_example["plan_digest"], r"^[a-f0-9]{64}$")

    def test_commands_file_contains_execute_example(self) -> None:
        commands = json.loads((PROTOCOL_DIR / "commands.json").read_text())
        self.assertEqual(commands["method_version"], 1)
        self.assertEqual(commands["protocol_version"], "1.0")
        self.assertEqual(commands["binary_path"], "/usr/libexec/sentia/sentia-apt")
        self.assertIn("apt_install_execute", commands["commands"])
        execute_cmd = commands["commands"]["apt_install_execute"]
        self.assertEqual(execute_cmd["arguments"]["mode"], "execute")
        self.assertIn("approval", execute_cmd)


if __name__ == "__main__":
    unittest.main()
