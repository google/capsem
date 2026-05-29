"""Regression test for the W3 IPC handshake.

Asserts that a client which connects to capsem-service's per-VM IPC
socket but does NOT send a Hello frame fails fast (within HELLO_TIMEOUT,
~5s) instead of hanging the connection forever the way the
pre-observability-sprint code did. Verifies the structured handshake
error is also emitted to service.log.
"""

import os
import socket
import time
from pathlib import Path

import pytest

pytestmark = pytest.mark.integration


def _per_vm_socket(service_env, vm_id: str) -> Path:
    """Resolve the per-VM IPC socket path (matches capsem-service's
    `{run_dir}/instances/{vm_id}.sock` layout)."""
    return Path(service_env.tmp_dir) / "instances" / f"{vm_id}.sock"


def test_pre_handshake_client_disconnects_quickly(service_env, fresh_vm):
    """A connection that opens the socket and never writes Hello must
    not pin a service worker indefinitely. Pre-W3: the v0 silent timeout
    pattern is exactly what kept investigations stuck."""
    name, _ = fresh_vm()
    # Wait briefly for the per-VM socket to appear (provision is async).
    sock_path = _per_vm_socket(service_env, name)
    deadline = time.time() + 10
    while time.time() < deadline and not sock_path.exists():
        time.sleep(0.1)

    if not sock_path.exists():
        pytest.skip(f"per-VM socket never appeared at {sock_path}")

    # Connect, immediately drop the connection without sending a Hello.
    # The service-side responder must time out via HELLO_TIMEOUT (5s),
    # log a structured error, and not leak the worker.
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(8.0)
    s.connect(str(sock_path))
    s.close()

    # The per-VM IPC responder lives in capsem-process, so the structured
    # handshake failure belongs in the VM's process.log rather than the
    # service log. Poll because the process can be CPU-starved in the n=4
    # integration gate.
    log_path = Path(service_env.tmp_dir) / "sessions" / name / "process.log"
    deadline = time.time() + 10
    text = ""
    while time.time() < deadline:
        if log_path.exists():
            text = log_path.read_text(errors="ignore")
            if '"target":"ipc"' in text or "IPC handshake failed" in text:
                break
        time.sleep(0.1)
    assert (
        '"target":"ipc"' in text or "IPC handshake failed" in text
    ), f"expected an ipc-targeted log line in {log_path}"
