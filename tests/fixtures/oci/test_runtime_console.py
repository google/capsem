"""The runtime hands Capsem the application's real PTY, without a second PTY."""

import array
import contextlib
import importlib.util
import os
import select
import socket
import subprocess
import sys
import tempfile
import tty
from pathlib import Path

import pytest

from tests.fixtures.oci.test_launch_config import SOURCE


@pytest.fixture
def runtime():
    spec = importlib.util.spec_from_file_location("runtime_console", SOURCE.with_name("runtime_console.py"))
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


@contextlib.contextmanager
def pair():
    sender, receiver = socket.socketpair()
    with sender, receiver:
        yield sender, receiver


def transfer(runtime, descriptors):
    with pair() as (sender, receiver):
        sender.sendmsg([b"/dev/pts/0"], [(socket.SOL_SOCKET, socket.SCM_RIGHTS, array.array("i", descriptors))])
        return runtime.receive_terminal(receiver)


def test_received_terminal_keeps_cooked_output_and_app_controls_raw_mode(runtime):
    master, slave = os.openpty()
    held = None
    try:
        held = transfer(runtime, [master])
        assert not os.get_inheritable(held)
        os.close(master)
        master = None
        os.write(slave, b"shell\n")
        assert select.select([held], [], [], 2)[0]
        assert os.read(held, 128) == b"shell\r\n"
        tty.setraw(slave)
        os.write(slave, b"abcdefgh\n\n\b\bX")
        assert os.read(held, 128) == b"abcdefgh\n\n\b\bX"
        os.write(held, b"\x1b[A\x1b[3;1R")
        assert select.select([slave], [], [], 2)[0]
        assert os.read(slave, 128) == b"\x1b[A\x1b[3;1R"
        assert not select.select([held], [], [], 0.05)[0], "raw input must not echo"
    finally:
        for fd in (held, master, slave):
            if fd is not None:
                os.close(fd)


@pytest.mark.parametrize("count", [0, 2, 8])
def test_handoff_refuses_missing_extra_or_truncated_descriptors(runtime, count):
    master, slave = os.openpty()
    try:
        with pair() as (sender, receiver):
            ancillary = [(socket.SOL_SOCKET, socket.SCM_RIGHTS, array.array("i", [master] * count))] if count else []
            sender.sendmsg([b"console"], ancillary)
            with pytest.raises(ValueError, match="one terminal"):
                runtime.receive_terminal(receiver)
        assert os.isatty(master), "refusal must close received copies, never the sender's descriptor"
    finally:
        os.close(master)
        os.close(slave)


def test_handoff_refuses_a_non_terminal_descriptor(runtime):
    with open(os.devnull, "rb") as source, pytest.raises(ValueError, match="terminal"):
        transfer(runtime, [source.fileno()])


def test_handoff_refuses_end_of_stream(runtime):
    with pair() as (sender, receiver):
        sender.close()
        with pytest.raises(ValueError, match="one terminal"):
            runtime.receive_terminal(receiver)


@pytest.mark.parametrize("script,error", [("raise SystemExit(17)", RuntimeError), ("import time; time.sleep(5)", TimeoutError)])
def test_failed_or_stalled_runtime_start_is_bounded_and_reaped(runtime, monkeypatch, tmp_path, script, error):
    # Startup/transport unit test only. Actual adoption and runc behavior are
    # proven by the real image-session Ironbank test, inside Linux.
    monkeypatch.setattr(runtime, "become_subreaper", lambda: None)
    owner = runtime.RuntimeConsole()
    with tempfile.TemporaryDirectory(prefix="cs", dir="/tmp") as directory:
        path = Path(directory) / "c.sock"
        with pytest.raises(error):
            owner.start([sys.executable, "-c", script], "workload", path, tmp_path / "pid", 0.2)
        owner.close()
        assert not path.exists()


def test_wait_preserves_exit_status_and_reaps_children_including_signal_death():
    # Keep the waitpid(-1) owner in a dedicated process, as in production;
    # it must never reap a test runner's unrelated children.
    script = f"""
import importlib.util, subprocess, sys
spec = importlib.util.spec_from_file_location('runtime_console', {str(SOURCE.with_name('runtime_console.py'))!r})
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
for command, expected in [('raise SystemExit(7)', 7), ('import os, signal; os.kill(os.getpid(), signal.SIGTERM)', 143)]:
    child = subprocess.Popen([sys.executable, '-c', command])
    owner = module.RuntimeConsole()
    owner.pid = child.pid
    assert owner.wait(timeout=5) == expected
    child.returncode = owner.returncode
    assert owner.wait(timeout=5) == expected
child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)'])
owner = module.RuntimeConsole()
owner.pid = child.pid
try:
    try:
        owner.wait(timeout=0.05)
    except subprocess.TimeoutExpired:
        pass
    else:
        raise AssertionError('wait must honor its deadline')
finally:
    child.kill()
    assert owner.wait(timeout=5) == 137
    child.returncode = owner.returncode
"""
    subprocess.run([sys.executable, "-c", script], check=True, timeout=10)
