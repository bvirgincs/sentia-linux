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
        self.assertIn("args", schema["properties"])
        approval_props = schema["properties"]["approval"]["properties"]
        self.assertIn("digest", approval_props)
        self.assertIn("sessionhash", approval_props)
        self.assertIn("planid", approval_props)
        self.assertIn("expires_utc", approval_props)

    def test_broker_contract_execute_requires_approval_fields(self) -> None:
        contract = json.loads((PROTOCOL_DIR / "broker-contract.json").read_text())
        self.assertEqual(contract["method_version"], 1)
        self.assertEqual(contract["protocol_version"], "1.0")
        self.assertEqual(contract["binary_path"], "/usr/libexec/sentia/sentia-apt-worker")
        self.assertIn(
            "/usr/libexec/sentia/sentia-apt",
            contract["binary_compatibility_paths"],
        )
        self.assertEqual(contract["runtime_limits"]["max_request_bytes"], 1048576)
        self.assertEqual(contract["runtime_limits"]["max_response_bytes"], 1048576)
        self.assertEqual(contract["runtime_limits"]["max_plan_retained_bytes"], 262144)
        self.assertEqual(contract["runtime_limits"]["plan_timeout_seconds"], 30)
        self.assertEqual(contract["runtime_limits"]["execute_timeout_seconds"], 900)
        self.assertTrue(contract["execution_environment"]["ignore_user_environment"])
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

    def test_broker_contract_external_adapter_mapping_present(self) -> None:
        contract = json.loads((PROTOCOL_DIR / "broker-contract.json").read_text())
        adapter = contract["external_operation_adapter"]
        self.assertEqual(
            adapter["external_shape"]["operation"],
            "apt_install|apt_remove|apt_update|apt_upgrade",
        )
        rules = adapter["mapping_rules"]
        self.assertIn(
            "for execution path broker emits arguments.mode=execute plus approval object",
            rules,
        )

    def test_commands_file_contains_execute_example(self) -> None:
        commands = json.loads((PROTOCOL_DIR / "commands.json").read_text())
        self.assertEqual(commands["method_version"], 1)
        self.assertEqual(commands["protocol_version"], "1.0")
        self.assertEqual(commands["binary_path"], "/usr/libexec/sentia/sentia-apt-worker")
        self.assertIn("apt_install_execute", commands["commands"])
        execute_cmd = commands["commands"]["apt_install_execute"]
        self.assertEqual(execute_cmd["arguments"]["mode"], "execute")
        self.assertIn("approval", execute_cmd)
        self.assertIn("apt_install_execute_compat_aliases", commands["commands"])
        compat = commands["commands"]["apt_install_execute_compat_aliases"]
        self.assertEqual(compat["args"]["mode"], "execute")
        self.assertIn("digest", compat["approval"])


if __name__ == "__main__":
    unittest.main()
