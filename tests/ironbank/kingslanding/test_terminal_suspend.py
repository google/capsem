"""A suspended image terminal resumes its actual foreground application."""

import hashlib
import shlex
import time

import msgpack
import pytest
from helpers.image_session import image_session, workload_exec
from helpers.service import vm_session_dir

from tests.ironbank.kingslanding.test_run import service
from tests.ironbank.kingslanding.test_terminal_state import next_state, terminal_summary
from tests.ironbank.kingslanding.test_workload_terminal import probe, terminal

__all__ = ["service"]
pytestmark = pytest.mark.integration


def display(socket, marker):
    seen = b""
    end = time.monotonic() + 10
    while time.monotonic() < end:
        frame = socket.recv(timeout=max(0.01, end - time.monotonic()))
        if isinstance(frame, bytes):
            seen += msgpack.unpackb(frame).get("stdout", b"")
            if marker in seen:
                return seen
    raise AssertionError(f"restored display lacks {marker!r}: {seen[-2000:]!r}")


def send(socket, data):
    socket.send(msgpack.packb({"stdin": data}, use_bin_type=True))


@pytest.mark.parametrize("fault", ["native-slot", "terminal-permissions"])
def test_failed_suspend_keeps_the_running_owner_and_foreground(service, tmp_path, fault):
    client = service.client()
    with image_session(service, tmp_path, "suspend-failure") as vm_id:
        original_pid = client.get(f"/vms/{vm_id}/info")["pid"]
        with terminal(service, vm_id) as socket:
            assert probe(socket) == ("1000", "100000", "workload")
            script = r"""
import os, termios, tty
before = termios.tcgetattr(0)
try:
    tty.setraw(0)
    os.write(1, b'\x1b]2;Suspend failure probe\x07FOREGROUND-READY\n\x07\x1b]777;notify;Rollback;Preserved\x07\x1b[38;2;10;')
    assert os.read(0, 1) == b'k'
    with open('/workspace/suspend-rollback-write', 'w') as stream:
        stream.write('foreground survived')
    os.write(1, b'20;30m\x1b]2;Foreground survived\x07FOREGROUND-SURVIVED\n')
finally:
    termios.tcsetattr(0, termios.TCSANOW, before)
"""
            send(socket, ("python3 -c " + shlex.quote(script) + "\n").encode())
            before = next_state(socket, lambda state: state.get("last_notification") is not None
                                and state["last_notification"]["title"] == "Rollback")
            display(socket, b"FOREGROUND-READY")
            # Host-owned faults at two stages of the same checkpoint pair:
            # the native slot is not removable, or terminal publication is
            # refused because the existing private file has unsafe mode.
            session = vm_session_dir(service.tmp_dir, client, vm_id)
            if fault == "native-slot":
                invalid_slot = session / "owner/checkpoint.vzsave"
                invalid_slot.mkdir()
            else:
                invalid_slot = session / "owner/terminal/checkpoint.msgpack"
                assert not invalid_slot.exists()
                invalid_slot.write_bytes(b"unsafe fixture mode")
                invalid_slot.chmod(0o644)
            try:
                status, error = client.call_json("POST", f"/vms/{vm_id}/pause", {}, timeout=60)
                assert status == 500, (status, error)
                assert client.get(f"/vms/{vm_id}/status")["status"] == "Running"
                assert client.get(f"/vms/{vm_id}/info")["pid"] == original_pid
                send(socket, b"k")
                after = next_state(socket, lambda state: state["title"] == "Foreground survived")
                assert after["bell_count"] == before["bell_count"]
                assert after["last_notification"] == before["last_notification"]
                assert b"38;2;10;20;30" in display(socket, b"FOREGROUND-SURVIVED")
                result = workload_exec(client, vm_id, "cat /workspace/suspend-rollback-write")
                assert result["exit_code"] == 0 and result["stdout_text"] == "foreground survived", result
                assert not (session / "owner/checkpoint.vzsave.complete").exists()
            finally:
                if fault == "native-slot":
                    invalid_slot.rmdir()
                else:
                    invalid_slot.unlink()
        # Correcting the fault must permit a new checkpoint on the same
        # owner, with fresh viewer/transport authority and actual guest thaw.
        assert client.post(f"/vms/{vm_id}/pause", {}, timeout=60) == {"success": True}
        assert client.get(f"/vms/{vm_id}/status")["status"] == "Suspended"


def test_failed_restore_preserves_suspended_state_without_cold_boot(service, tmp_path):
    client = service.client()
    with image_session(service, tmp_path, "restore-failure") as vm_id:
        assert client.post(f"/vms/{vm_id}/pause", {}, timeout=60) == {"success": True}
        assert client.get(f"/vms/{vm_id}/status")["status"] == "Suspended"
        session = vm_session_dir(service.tmp_dir, client, vm_id)
        checkpoint = session / "owner/checkpoint.vzsave"
        complete = checkpoint.with_suffix(".vzsave.complete")
        terminal_checkpoint = session / "owner/terminal/checkpoint.msgpack"
        completed = complete.read_bytes()
        terminal_state = terminal_checkpoint.read_bytes()
        original_size = checkpoint.stat().st_size
        # A real native restore must reject this damaged header. A cold boot
        # would hide the error and replace the suspended foreground process.
        damaged_header = b"CAPSEM: intentionally damaged VM checkpoint\n"
        with checkpoint.open("r+b") as stream:
            stream.write(damaged_header)
        with checkpoint.open("rb") as stream:
            damaged_digest = hashlib.file_digest(stream, "sha256").digest()

        status, error = client.call_json("POST", f"/vms/{vm_id}/resume", {}, timeout=90)
        assert status == 500, (status, error)
        assert "resume failed" in error["error"], error
        assert client.get(f"/vms/{vm_id}/status")["status"] == "Suspended"
        listed = next(row for row in client.get("/vms/list")["sandboxes"] if row["id"] == vm_id)
        assert listed["status"] == "Suspended", listed
        assert checkpoint.stat().st_size == original_size
        with checkpoint.open("rb") as stream:
            assert stream.read(len(damaged_header)) == damaged_header
        with checkpoint.open("rb") as stream:
            assert hashlib.file_digest(stream, "sha256").digest() == damaged_digest
        assert complete.read_bytes() == completed
        assert terminal_checkpoint.read_bytes() == terminal_state
        assert not list(checkpoint.parent.glob("*.failed-restore-*"))


def test_suspend_restores_both_screens_parser_modes_and_effect_identity(service, tmp_path):
    client = service.client()
    with image_session(service, tmp_path, "terminal-suspend") as vm_id:
        original = client.get(f"/vms/{vm_id}/info")
        assert original["persistent"] is True, original
        with terminal(service, vm_id) as socket:
            assert probe(socket) == ("1000", "100000", "workload")
            baseline = next(row for row in client.get("/vms/list")["sandboxes"] if row["id"] == vm_id)["terminal"]
            # The app waits in raw mode throughout suspend. It leaves an
            # unfinished CSI, two saved cursors, both screens and one alert.
            script = r"""
import os, termios, tty
from pathlib import Path
before = termios.tcgetattr(0)
try:
    tty.setraw(0)
    Path('/workspace/terminal-suspend-pid').write_text(str(os.getpid()))
    os.write(1, b'\x1b[0m\x1b[2J\x1b[HPRIMARY-DISPLAY\x1b[4;6H\x1b7')
    os.write(1, b'\x1b[?1049h\x1b[2J\x1b[H\x1b[38;2;23;89;144mALTERNATE-DISPLAY\x1b[4;8H\x1b7')
    os.write(1, b'\x1b[?1h\x1b[?2004h\x07\x1b]777;notify;Suspend;Retained\x07\x1b]0;Persisted terminal\x07\x1b[38;2;10;')
    assert os.read(0, 1) == b'c'
    os.write(1, b'20;30mTAIL\x1b8X\x1b]2;Alternate restored\x07')
    assert os.read(0, 1) == b'p'
    os.write(1, b'\x1b[?1049l\x1b8Y\x1b]2;Primary restored\x07')
    assert os.read(0, 1) == b'q'
finally:
    os.write(1, b'\x1b[?1049l\x1b[?1l\x1b[?2004l')
    termios.tcsetattr(0, termios.TCSANOW, before)
"""
            send(socket, ("python3 -c " + shlex.quote(script) + "\n").encode())
            before = next_state(socket, lambda state: state["title"] == "Persisted terminal")
            assert before["modes"]["alternate_screen"] is True
            assert before["modes"]["application_cursor"] is True
            assert before["modes"]["bracketed_paste"] is True
            assert (before["cursor"]["row"], before["cursor"]["col"]) == (3, 7), before
            assert before["bell_count"] == baseline["bell_count"] + 1
            assert before["last_notification"]["title"] == "Suspend"
            assert before["last_notification"]["body"] == "Retained"
            assert b"38;2;23;89;144" in display(socket, b"ALTERNATE-DISPLAY")
            summary = terminal_summary(client, vm_id, "Persisted terminal")
            foreground = workload_exec(client, vm_id, "cat /workspace/terminal-suspend-pid")
            assert foreground["exit_code"] == 0, foreground

        paused = client.post(f"/vms/{vm_id}/pause", {}, timeout=60)
        assert paused == {"success": True}, paused
        assert client.get(f"/vms/{vm_id}/status")["status"] == "Suspended"
        resumed = client.post(f"/vms/{vm_id}/resume", {}, timeout=90)
        assert resumed["id"] == vm_id, resumed
        # The first public exec proves the resumed workload is usable. Its
        # foreground PID plus subsequent key-driven stages rule out cold boot.
        alive = workload_exec(client, vm_id, "pid=$(cat /workspace/terminal-suspend-pid); kill -0 $pid; printf '%s' \"$pid\"")
        assert alive["exit_code"] == 0 and alive["stdout_text"] == foreground["stdout_text"], alive
        assert client.get(f"/vms/{vm_id}/info")["pid"] != original["pid"]
        retained = terminal_summary(client, vm_id, "Persisted terminal")
        for field in ["title", "bell_count", "last_notification"]:
            assert retained[field] == summary[field], (field, retained, summary)

        with terminal(service, vm_id) as socket:
            restored = next_state(socket, lambda state: state["reset"])
            for field in ["cols", "rows", "cursor", "modes", "title", "icon_name", "bell_count", "last_notification"]:
                assert restored[field] == before[field], (field, restored, before)
            assert restored["events"] == [], "restore replayed a bell or notification"
            assert b"38;2;23;89;144" in display(socket, b"ALTERNATE-DISPLAY")
            send(socket, b"c")
            alternate = next_state(socket, lambda state: state["title"] == "Alternate restored")
            assert (alternate["cursor"]["row"], alternate["cursor"]["col"]) == (3, 8), alternate
            assert b"38;2;10;20;30" in display(socket, b"AIL")
            send(socket, b"p")
            primary = next_state(socket, lambda state: state["title"] == "Primary restored")
            assert primary["modes"]["alternate_screen"] is False
            assert (primary["cursor"]["row"], primary["cursor"]["col"]) == (3, 6), primary
            display(socket, b"PRIMARY-DISPLAY")
            assert primary["bell_count"] == before["bell_count"]
            assert primary["last_notification"] == before["last_notification"]
            send(socket, b"q")
