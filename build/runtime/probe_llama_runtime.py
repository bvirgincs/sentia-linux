#!/usr/bin/env python3
"""Probe a running llama-server over Unix socket for health and generation."""

from __future__ import annotations

import argparse
import http.client
import json
import os
import socket
import time
from pathlib import Path


class UnixHTTPConnection(http.client.HTTPConnection):
    def __init__(self, unix_socket_path: str):
        super().__init__("localhost")
        self.unix_socket_path = unix_socket_path

    def connect(self) -> None:
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.connect(self.unix_socket_path)


def request_json(socket_path: str, method: str, path: str, payload: dict | None = None) -> tuple[int, object]:
    body = None
    headers = {}
    if payload is not None:
        body = json.dumps(payload).encode("utf-8")
        headers["Content-Type"] = "application/json"

    conn = UnixHTTPConnection(socket_path)
    conn.request(method, path, body=body, headers=headers)
    response = conn.getresponse()
    raw = response.read()
    conn.close()

    text = raw.decode("utf-8", errors="replace")
    try:
        return response.status, json.loads(text)
    except json.JSONDecodeError:
        return response.status, text


def read_vm_hwm_kib(pid: int) -> int | None:
    status_path = Path(f"/proc/{pid}/status")
    if not status_path.is_file():
        return None
    for line in status_path.read_text(encoding="utf-8", errors="ignore").splitlines():
        if line.startswith("VmHWM:"):
            parts = line.split()
            if len(parts) >= 2 and parts[1].isdigit():
                return int(parts[1])
    return None


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--socket", required=True)
    parser.add_argument("--max-wait-seconds", type=int, default=180)
    parser.add_argument("--n-predict", type=int, default=16)
    parser.add_argument("--prompt", default="Reply with exactly: OK")
    parser.add_argument("--sleep-check-seconds", type=int, default=0)
    parser.add_argument("--server-pid", type=int, default=0)
    parser.add_argument("--report-path", default="")
    args = parser.parse_args()

    started = time.time()
    health_ready = False
    health_attempts = 0
    last_health_response = None

    while time.time() - started < args.max_wait_seconds:
        health_attempts += 1
        try:
            status, body = request_json(args.socket, "GET", "/health")
            last_health_response = {"status": status, "body": body}
            if status == 200 and isinstance(body, dict) and body.get("status") == "ok":
                health_ready = True
                break
        except OSError as exc:
            last_health_response = {"error": str(exc)}
        time.sleep(1)

    report: dict[str, object] = {
        "socket": args.socket,
        "max_wait_seconds": args.max_wait_seconds,
        "health_attempts": health_attempts,
        "health_ready": health_ready,
        "last_health_response": last_health_response,
    }

    if not health_ready:
        if args.report_path:
            Path(args.report_path).parent.mkdir(parents=True, exist_ok=True)
            Path(args.report_path).write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        return 1

    t0 = time.time()
    status, completion = request_json(
        args.socket,
        "POST",
        "/completion",
        {
            "prompt": args.prompt,
            "n_predict": args.n_predict,
            "temperature": 0,
        },
    )
    generation_seconds = time.time() - t0

    completion_ok = status == 200 and isinstance(completion, dict) and bool(completion.get("content"))
    report["generation"] = {
        "status": status,
        "seconds": generation_seconds,
        "response": completion,
        "ok": completion_ok,
    }

    if args.sleep_check_seconds > 0:
        time.sleep(args.sleep_check_seconds)
        s_status, props = request_json(args.socket, "GET", "/props")
        report["sleep_state"] = {
            "status": s_status,
            "response": props,
            "is_sleeping": isinstance(props, dict) and bool(props.get("is_sleeping")),
        }

    if args.server_pid > 0:
        report["memory_vm_hwm_kib"] = read_vm_hwm_kib(args.server_pid)

    report["success"] = completion_ok

    if args.report_path:
        report_path = Path(args.report_path)
        report_path.parent.mkdir(parents=True, exist_ok=True)
        report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")

    return 0 if completion_ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
