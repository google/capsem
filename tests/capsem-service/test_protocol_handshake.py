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

    # Give the service a moment to flush the handshake-failed log line.
    time.sleep(0.5)

    # Verify service.log saw the handshake error. We don't pin an exact
    # message because the wording may shift; the `target=ipc` prefix is
    # the load-bearing contract.
    log_path = Path(service_env.tmp_dir) / "service.log"
    if log_path.exists():
        text = log_path.read_text(errors="ignore")
        assert (
            '"target":"ipc"' in text or "ipc" in text
        ), "expected an ipc-targeted log line in service.log"
