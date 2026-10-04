"""The launcher's held console: a detached workload's terminal.

An image's command is often interactive -- a shell, an agent's TUI. In a
detached session nothing is attached when it starts, so the launcher runs it
on a PTY it keeps (`docker run -dit`): output is drained into a replay buffer
so the workload never blocks, and the session terminal attaches to that same
PTY through a socket, input, resizes and all. When the command exits the
workload has exited, and the stage says so with its code.
"""

import contextlib
import fcntl
import os
import socket
import struct
import tempfile
import termios
import threading
import time
import tty
from pathlib import Path

import pytest

from tests.fixtures.oci.test_launch_config import launcher

__all__ = ["launcher"]


def test_input_and_resize_frames_survive_any_read_boundary(launcher):
    stream = (
        launcher.encode_input(b"ls -la\n")
        + launcher.encode_resize(40, 120)
        + launcher.encode_input(b"")
    )
    reader = launcher.FrameReader()
    events = [event for byte in stream for event in reader.feed(bytes([byte]))]
    assert events == [("input", b"ls -la\n"), ("resize", (40, 120)), ("input", b"")]


def test_a_malformed_frame_is_refused(launcher):
    with pytest.raises(ValueError, match="frame"):
        launcher.FrameReader().feed(b"x\x00\x00\x00\x01a")


def _winsize(fd):
    rows, cols, _, _ = struct.unpack(
        "HHHH", fcntl.ioctl(fd, termios.TIOCGWINSZ, b"\0" * 8)
    )
    return rows, cols


def _read_until(sock, wanted, timeout=5.0):
    sock.settimeout(timeout)
    received = b""
    deadline = time.monotonic() + timeout
    while wanted not in received and time.monotonic() < deadline:
        chunk = sock.recv(4096)
        if not chunk:
            break
        received += chunk
    return received


@pytest.fixture
def held(launcher):
    """A console holding a real PTY's master, serving on a socket; yields
    (console, slave fd, socket path, server thread, resize callbacks)."""
    master, slave = os.openpty()
    tty.setraw(slave)
    # AF_UNIX paths are short (104 bytes on macOS); pytest's tmp_path is not.
    path = Path(tempfile.mkdtemp(prefix="cs", dir="/tmp")) / "c.sock"
    listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    listener.bind(str(path))
    listener.listen()
    resizes = []
    console = launcher.Console(
        master, listener, replay_limit=64, resized=lambda: resizes.append(1)
    )
    server = threading.Thread(target=console.serve, daemon=True)
    server.start()
    yield console, slave, path, server, resizes
    with contextlib.suppress(OSError):
        os.close(slave)
    server.join(timeout=5)
    listener.close()
    with contextlib.suppress(OSError):
        os.close(master)


def test_output_before_an_attach_is_replayed_then_streamed_live(held):
    console, slave, path, _, _ = held
    os.write(slave, b"early")
    deadline = time.monotonic() + 5
    while console.replayed() != b"early" and time.monotonic() < deadline:
        time.sleep(0.01)
    with socket.socket(socket.AF_UNIX) as client:
        client.connect(str(path))
        assert _read_until(client, b"early").endswith(b"early")
        os.write(slave, b"live")
        assert _read_until(client, b"live").endswith(b"live")


def test_the_replay_keeps_only_the_newest_output(held):
    console, slave, _, _, _ = held
    os.write(slave, b"a" * 100 + b"tail")
    deadline = time.monotonic() + 5
    while not console.replayed().endswith(b"tail") and time.monotonic() < deadline:
        time.sleep(0.01)
    assert len(console.replayed()) == 64 and console.replayed().endswith(b"tail")


def test_an_attached_terminal_types_and_resizes_the_workload(launcher, held):
    _, slave, path, _, _ = held
    with socket.socket(socket.AF_UNIX) as client:
        client.connect(str(path))
        client.sendall(
            launcher.encode_resize(33, 101) + launcher.encode_input(b"typed")
        )
        received, deadline = b"", time.monotonic() + 5
        os.set_blocking(slave, False)
        while received != b"typed" and time.monotonic() < deadline:
            try:
                received += os.read(slave, 64)
            except BlockingIOError:
                time.sleep(0.01)
        assert received == b"typed"
        assert _winsize(slave) == (33, 101)
        # runc copies the size to the workload only when told.
        assert held[4] == [1]


def test_a_departed_terminal_leaves_the_workload_running(held):
    console, slave, path, server, _ = held
    with socket.socket(socket.AF_UNIX) as client:
        client.connect(str(path))
    os.write(slave, b"still here")
    deadline = time.monotonic() + 5
    while (
        not console.replayed().endswith(b"still here") and time.monotonic() < deadline
    ):
        time.sleep(0.01)
    assert console.replayed().endswith(b"still here")
    assert server.is_alive()


def test_the_console_ends_with_the_workload_and_tells_the_terminal(held):
    _, slave, path, server, _ = held
    with socket.socket(socket.AF_UNIX) as client:
        client.connect(str(path))
        time.sleep(0.1)
        os.close(slave)
        server.join(timeout=5)
        assert not server.is_alive()
        client.settimeout(5)
        assert client.recv(4096) == b""


def test_a_workload_that_started_and_ended_is_exited_with_its_code(launcher, tmp_path):
    (tmp_path / launcher.RUNNING).write_text("1\n")
    launcher.launch_ended(tmp_path, 3)
    assert (tmp_path / launcher.EXITED).read_text() == "3\n"
    assert not (tmp_path / launcher.FAILED).exists()


def test_a_workload_that_never_started_is_failed(launcher, tmp_path):
    launcher.launch_ended(tmp_path, 1)
    assert (tmp_path / launcher.FAILED).is_file()
    assert not (tmp_path / launcher.EXITED).exists()


def test_a_new_launch_forgets_how_the_last_one_ended(launcher, tmp_path):
    for marker in (launcher.RUNNING, launcher.FAILED, launcher.EXITED):
        (tmp_path / marker).write_text("1\n")
    launcher.clear_launch_markers(tmp_path)
    assert not any(
        (tmp_path / marker).exists()
        for marker in (launcher.RUNNING, launcher.FAILED, launcher.EXITED)
    )


def test_a_detached_launch_runs_its_command_on_a_terminal(launcher):
    from tests.fixtures.oci.test_launch_config import SECURITY, image, unpacked

    attached = launcher.configure(
        unpacked(), image(), {**SECURITY, "args": [], "env": {}}
    )
    detached = launcher.configure(
        unpacked(), image(), {**SECURITY, "args": [], "env": {}}, detached=True
    )
    assert attached["process"]["terminal"] is False
    assert detached["process"]["terminal"] is True
    assert launcher.TERMINAL_TYPE in detached["process"]["env"]


def test_the_terminal_relays_keys_out_and_output_in_until_the_workload_ends(launcher):
    """`--attach` on a held console: keystrokes become input frames, output is
    copied verbatim, and the relay returns when the console closes."""
    near, far = socket.socketpair()
    keys_read, keys_write = os.pipe()
    out_read, out_write = os.pipe()
    relay = threading.Thread(
        target=launcher.relay, args=(near, keys_read, out_write), daemon=True
    )
    relay.start()
    os.write(keys_write, b"q")
    reader, frames = launcher.FrameReader(), []
    far.settimeout(5)
    while ("input", b"q") not in frames:
        frames += reader.feed(far.recv(64))
    far.sendall(b"screen")
    assert os.read(out_read, 64) == b"screen"
    far.close()
    relay.join(timeout=5)
    assert not relay.is_alive()
    near.close()
    for fd in (keys_read, keys_write, out_read, out_write):
        os.close(fd)
