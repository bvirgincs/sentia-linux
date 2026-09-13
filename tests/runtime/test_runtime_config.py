import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
SERVICE = ROOT / "config/systemd/sentia-local-llama.service"
READINESS = ROOT / "config/systemd/sentia-local-llama-readiness.service"
ENV_FILE = ROOT / "config/llama/sentia-local.env"


class RuntimeConfigTests(unittest.TestCase):
    def test_service_hardening_and_user(self) -> None:
        text = SERVICE.read_text(encoding="utf-8")
        for needle in [
            "User=sentia-inference",
            "Group=sentia-inference",
            "RestrictAddressFamilies=AF_UNIX",
            "IPAddressDeny=any",
            "PrivateNetwork=yes",
            "MemoryHigh=8G",
            "MemoryMax=10G",
        ]:
            self.assertIn(needle, text)

    def test_service_runtime_flags(self) -> None:
        text = SERVICE.read_text(encoding="utf-8")
        for needle in [
            "--host ${SENTIA_SOCKET_PATH}",
            "--ctx-size ${SENTIA_CTX_SIZE}",
            "--parallel ${SENTIA_N_PARALLEL}",
            "--predict ${SENTIA_MAX_PREDICT}",
            "--sleep-idle-seconds ${SENTIA_SLEEP_IDLE_SECONDS}",
            "--no-webui",
            "--metrics",
            "--no-slots",
        ]:
            self.assertIn(needle, text)

    def test_service_does_not_enable_tools_or_remote_download(self) -> None:
        text = SERVICE.read_text(encoding="utf-8")
        forbidden = ["--tools", "--agent", "--mcp-servers", "--hf-repo", "--model-url"]
        for flag in forbidden:
            self.assertNotIn(flag, text)

    def test_env_bounds(self) -> None:
        env_text = ENV_FILE.read_text(encoding="utf-8")
        expected = {
            "SENTIA_CTX_SIZE": "8192",
            "SENTIA_N_PARALLEL": "1",
            "SENTIA_MAX_PREDICT": "256",
            "SENTIA_SLEEP_IDLE_SECONDS": "300",
            "SENTIA_SOCKET_PATH": "/run/sentia-local/llama.sock",
        }
        kv = {}
        for line in env_text.splitlines():
            if not line or line.startswith("#") or "=" not in line:
                continue
            key, value = line.split("=", 1)
            kv[key] = value
        for key, value in expected.items():
            self.assertEqual(kv.get(key), value)

    def test_readiness_probe_uses_unix_socket(self) -> None:
        text = READINESS.read_text(encoding="utf-8")
        self.assertIn("--socket ${SENTIA_SOCKET_PATH}", text)
        self.assertIn("/usr/lib/sentia/runtime/probe_llama_runtime.sh", text)


if __name__ == "__main__":
    unittest.main()
