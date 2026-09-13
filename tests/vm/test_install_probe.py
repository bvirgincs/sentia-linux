# SPDX-License-Identifier: Apache-2.0
"""Guest shell parsing, tested against a getty that echoes what it is sent.

The first real installation attempt failed with "graphical.target never became
active" while the serial log showed the guest answering YES to that exact
question every five seconds. The markers delimiting a command's output were
matching the terminal's echo of the command line instead of the output, so
every command looked empty.
"""
import importlib.util
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("install_probe.py")
SPEC = importlib.util.spec_from_file_location("install_probe", MODULE_PATH)
assert SPEC and SPEC.loader
install_probe = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(install_probe)


class EchoingSession:
    """A getty that echoes the command line, then runs a canned shell."""

    def __init__(self, outputs: list[str]) -> None:
        self._buffer = ""
        self._outputs = list(outputs)

    @property
    def transcript(self) -> str:
        return self._buffer

    def send(self, line: str) -> None:
        self._buffer += f"user@sentia:~$ {line}\n"
        emitted = self._outputs.pop(0) if self._outputs else ""
        # The shell resolves the quoting, so the markers it prints are whole.
        self._buffer += line.replace("''", "").replace(
            "echo ", ""
        ).split(";")[0].strip() + "\n"
        if emitted:
            self._buffer += f"{emitted}\n"
        end = line.rsplit("echo ", 1)[-1].replace("''", "").replace("$?", "")
        self._buffer += f"{end}0\n"

    def wait_for(self, needle: str, timeout: float) -> bool:
        return needle in self._buffer


class GuestShellTests(unittest.TestCase):
    def test_output_is_not_confused_with_the_echoed_command(self):
        guest = install_probe.Guest(EchoingSession(["YES"]))
        self.assertEqual(guest.run("systemctl is-active graphical.target"), "YES")

    def test_multiple_commands_keep_distinct_markers(self):
        session = EchoingSession(["first", "second"])
        guest = install_probe.Guest(session)
        self.assertEqual(guest.run("one"), "first")
        self.assertEqual(guest.run("two"), "second")

    def test_wait_until_sees_a_successful_predicate(self):
        guest = install_probe.Guest(EchoingSession(["YES"]))
        self.assertTrue(guest.wait_until("systemctl is-active graphical.target", 5))

    def test_background_command_is_braced(self):
        session = EchoingSession(["started"])
        guest = install_probe.Guest(session)
        guest.launch("setsid /usr/libexec/sentia-live/run-calamares-root -d &")
        typed = session.transcript
        self.assertIn("{ setsid /usr/libexec/sentia-live/run-calamares-root -d & }", typed)
        self.assertNotIn("& ;", typed)

    def test_marker_is_typed_in_two_halves(self):
        typed = install_probe.split_marker("SENTIA_CMD_1_BEGIN")
        self.assertNotIn("SENTIA_CMD_1_BEGIN", typed)
        self.assertEqual(typed.replace("''", ""), "SENTIA_CMD_1_BEGIN")


if __name__ == "__main__":
    unittest.main()
