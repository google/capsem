"""Rejected native suspend preserves the terminal's accepted byte stream."""

import hashlib
import shlex

import pytest
from helpers.image_session import image_session, workload_exec
from helpers.service import vm_session_dir

from tests.ironbank.kingslanding.test_run import service
from tests.ironbank.kingslanding.test_terminal_state import next_state
from tests.ironbank.kingslanding.test_terminal_suspend import display, send
from tests.ironbank.kingslanding.test_workload_terminal import probe, terminal

__all__ = ["service"]
pytestmark = pytest.mark.integration


def test_suspend_rollback_delivers_queued_input_exactly_once(service, tmp_path):
    client = service.client()
    payload = hashlib.shake_256(b"Capsem queued suspend rollback").digest(2 * 1024 * 1024)
    with image_session(service, tmp_path, "queued-suspend-input") as vm_id:
        original_pid = client.get(f"/vms/{vm_id}/info")["pid"]
        with terminal(service, vm_id) as socket:
            assert probe(socket) == ("1000", "100000", "workload")
            script = r"""
import array, fcntl, hashlib, os, termios, time, tty
from pathlib import Path
before = termios.tcgetattr(0)
try:
    tty.setraw(0)
    os.write(1, b'\x1b]2;Waiting for input\x07INPUT-WAIT\n')
    queued = False
    deadline = time.monotonic() + 60
    while not Path('/workspace/read-terminal-input').exists():
        assert time.monotonic() < deadline
        available = array.array('i', [0])
        fcntl.ioctl(0, termios.FIONREAD, available, True)
        if available[0] and not queued:
            queued = True
            os.write(1, b'\x1b]2;Input queued\x07INPUT-QUEUED\n')
        time.sleep(.01)
    remaining = 2 * 1024 * 1024
    digest = hashlib.sha256()
    while remaining:
        data = os.read(0, min(65536, remaining))
        assert data
        digest.update(data)
        remaining -= len(data)
    Path('/workspace/queued-input-digest').write_text(digest.hexdigest())
    os.write(1, b'\x1b]2;Input survived\x07INPUT-SURVIVED\n')
    assert os.read(0, 1) == b'q'
    os.write(1, b'\x1b]2;Input complete\x07INPUT-COMPLETE\n')
    assert os.read(0, 1) == b'x'
finally:
    termios.tcsetattr(0, termios.TCSANOW, before)
"""
            send(socket, ("python3 -c " + shlex.quote(script) + "\n").encode())
            next_state(socket, lambda state: state["title"] == "Waiting for input")
            session = vm_session_dir(service.tmp_dir, client, vm_id)
            fault = session / "owner/terminal/checkpoint.msgpack"
            assert not fault.exists()
            fault.write_bytes(b"unsafe fixture mode")
            fault.chmod(0o644)
            try:
                for offset in range(0, len(payload), 64 * 1024):
                    send(socket, payload[offset:offset + 64 * 1024])
                # FIONREAD confirms real guest PTY input without consuming it.
                # The foreground cannot drain it until the independent exec.
                next_state(socket, lambda state: state["title"] == "Input queued")
                status, error = client.call_json("POST", f"/vms/{vm_id}/pause", {}, timeout=60)
                assert status == 501 and "warm suspend is disabled" in error["error"], (status, error)
                assert client.get(f"/vms/{vm_id}/status")["status"] == "Running"
                assert client.get(f"/vms/{vm_id}/info")["pid"] == original_pid
                released = workload_exec(client, vm_id, "touch /workspace/read-terminal-input")
                assert released["exit_code"] == 0, released
                next_state(socket, lambda state: state["title"] == "Input survived")
                display(socket, b"INPUT-SURVIVED")
                received = workload_exec(client, vm_id, "cat /workspace/queued-input-digest")
                assert received["exit_code"] == 0, received
                assert received["stdout_text"] == hashlib.sha256(payload).hexdigest(), received
                assert not (session / "owner/checkpoint.vzsave.complete").exists()
                send(socket, b"q")
                next_state(socket, lambda state: state["title"] == "Input complete")
                display(socket, b"INPUT-COMPLETE")
                send(socket, b"x")
            finally:
                fault.unlink()
