# SPDX-License-Identifier: Apache-2.0
import importlib.util
import json
import tempfile
import unittest
from argparse import Namespace
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("vm_harness.py")
SPEC = importlib.util.spec_from_file_location("vm_harness", MODULE_PATH)
assert SPEC and SPEC.loader
vm_harness = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(vm_harness)


class HarnessTests(unittest.TestCase):
    def test_rejects_host_block_device_namespace(self):
        with self.assertRaises(vm_harness.HarnessError):
            vm_harness.regular_image(Path("/dev/sda"))

    def test_loads_bounded_scenario_actions(self):
        with tempfile.TemporaryDirectory(dir=Path.cwd()) as directory:
            scenario = Path(directory) / "scenario.json"
            scenario.write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "steps": [
                            {"action": "wait", "seconds": 1},
                            {"action": "screenshot", "name": "ok.ppm"},
                        ],
                    }
                )
            )
            loaded = vm_harness.load_scenario(scenario)
            self.assertEqual(len(loaded["steps"]), 2)

    def test_rejects_unrestricted_qmp_action(self):
        with tempfile.TemporaryDirectory(dir=Path.cwd()) as directory:
            scenario = Path(directory) / "scenario.json"
            scenario.write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "steps": [{"action": "human-monitor-command"}],
                    }
                )
            )
            with self.assertRaises(vm_harness.HarnessError):
                vm_harness.load_scenario(scenario)

    def test_uppercase_text_holds_shift(self):
        events = vm_harness.character_events("A")
        self.assertEqual(events[0]["data"]["key"]["data"], "shift")
        self.assertTrue(events[0]["data"]["down"])
        self.assertEqual(events[1]["data"]["key"]["data"], "a")
        self.assertFalse(events[-1]["data"]["down"])

    def test_rejects_unsupported_text(self):
        with self.assertRaises(vm_harness.HarnessError):
            vm_harness.character_events("€")

    def test_production_image_rejects_test_authentication(self):
        args = Namespace(
            mode="boot-iso",
            iso="sentia.iso",
            disk=None,
            scenario=None,
            image_kind="production",
            test_ssh_key="test.key",
            ssh_user="tester",
            success_serial_regex=["ready"],
            cpus=2,
            memory_mib=2048,
            ssh_port=2222,
            accel="tcg",
        )
        with self.assertRaises(vm_harness.HarnessError):
            vm_harness.validate_args(args)

    def test_install_requires_real_scenario(self):
        args = Namespace(
            mode="install",
            iso="sentia.iso",
            disk=None,
            scenario=None,
            image_kind="production",
            test_ssh_key=None,
            ssh_user=None,
            success_serial_regex=["ready"],
            cpus=2,
            memory_mib=2048,
            ssh_port=2222,
            accel="tcg",
        )
        with self.assertRaises(vm_harness.HarnessError):
            vm_harness.validate_args(args)


if __name__ == "__main__":
    unittest.main()
