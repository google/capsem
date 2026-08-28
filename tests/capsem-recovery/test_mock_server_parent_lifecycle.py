"""Cross-process acceptance for the mock server's parent-death contract."""

from __future__ import annotations

import socket
import subprocess
import sys
from pathlib import Path

from helpers.mock_server import start_mock_server, stop_process

PROJECT_ROOT = Path(__file__).resolve().parents[2]


def test_mock_server_exits_when_its_launcher_dies() -> None:
    holder = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    holder.bind(("127.0.0.1", 0))
    host, port = holder.getsockname()
    holder.close()
    addr = f"{host}:{port}"
    seeded = None
    try:
        seeded, _ready = start_mock_server(addr=addr, timeout_s=5)
    finally:
        stop_process(seeded)
    launcher_code = """
import os
import sys
from scripts.mock_server import start_mock_server

child, _ready = start_mock_server(addr=sys.argv[1], timeout_s=5)
print(child.pid, flush=True)
os._exit(0)
"""
    launcher = subprocess.Popen(
        [sys.executable, "-c", launcher_code, addr],
        cwd=PROJECT_ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    try:
        stdout, stderr = launcher.communicate(timeout=30)
    except subprocess.TimeoutExpired:
        launcher.kill()
        launcher.communicate()
        raise
    assert launcher.returncode == 0, stderr
    orphan_pid = int(stdout.strip())

    replacement = None
    try:
        replacement, ready = start_mock_server(
            addr=addr,
            timeout_s=5,
            retry_interval_s=0.05,
        )
        assert replacement.pid != orphan_pid
        assert ready["base_url"] == f"http://{addr}"
    finally:
        stop_process(replacement)
