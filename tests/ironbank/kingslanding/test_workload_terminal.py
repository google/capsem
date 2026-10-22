"""An image session's terminal is its workload's own command, never the VM.

`capsem create --image` marks the session (capsem-core `WORKLOAD_ENV`), so the
VM's terminal shell hands its PTY to the launcher's attach loop before any VM
prompt. A detached workload runs on a terminal the launcher holds
(`docker run -dit`), and the session terminal attaches to it: opening a
session of the reference image (the official `dev`, whose command is bash)
shows that bash, as the image's own user inside the workload's user
namespace. Leaving the terminal leaves the workload running, and attaching
again shows where it was. When the command exits, the workload has exited.
"""

import contextlib
import re
import time

import msgpack
import pytest
from helpers.image_session import image_session
from websockets.sync.client import connect
from websockets.typing import Subprotocol

from tests.ironbank.kingslanding.test_run import service

__all__ = ["service"]

pytestmark = pytest.mark.integration

# Typed into the terminal. The echoed line holds `$(id -u)` unexpanded, so only
# the shell's own output matches PROBE.
TYPED = 'read _ lower _ < /proc/self/uid_map; echo "PROBE-$(id -u)-$lower-$([ -d /var/tmp/capsem-container ] && echo vm || echo workload)"\n'
PROBE = re.compile(r"PROBE-(\d+)-(\d+)-(vm|workload)")


@contextlib.contextmanager
def terminal(service, vm_id):
    port = int((service.tmp_dir / "gateway.port").read_text())
    token = (service.tmp_dir / "gateway.token").read_text().strip()
    with connect(
        f"ws://127.0.0.1:{port}/vms/{vm_id}/stream",
        subprotocols=[Subprotocol("capsem.stream.v3")],
        additional_headers={"Authorization": f"Bearer {token}"},
        open_timeout=10,
    ) as socket:
        socket.send(
            msgpack.packb({"control": {"type": "start", "kind": "terminal"}}, use_bin_type=True)
        )
        socket.send(msgpack.packb({"control": {"type": "focus", "cols": 80, "rows": 24}}, use_bin_type=True))
        yield socket


def probe(socket, after=b"", deadline=90):
    """Type the probe until the terminal answers it after `after`; the first
    attempts may land while the attach loop is still waiting for the workload."""
    seen = b""
    end = time.monotonic() + deadline
    next_send = 0.0
    while time.monotonic() < end:
        if time.monotonic() >= next_send:
            socket.send(msgpack.packb({"stdin": TYPED.encode()}, use_bin_type=True))
            next_send = time.monotonic() + 5
        try:
            frame = socket.recv(timeout=1)
        except TimeoutError:
            continue
        if isinstance(frame, bytes):
            message = msgpack.unpackb(frame)
            seen += message.get("stdout", message.get("stderr", b""))
            start = seen.find(after)
            found = start >= 0 and PROBE.search(
                seen[start + len(after) :].decode(errors="replace")
            )
            if found:
                return found.groups()
    raise AssertionError(f"the terminal never answered the probe: {seen[-2000:]!r}")


def expect_output(socket, expected, timeout=10):
    seen = b""
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        try:
            frame = socket.recv(timeout=max(0.01, end - time.monotonic()))
        except TimeoutError:
            break
        if isinstance(frame, bytes):
            message = msgpack.unpackb(frame)
            seen += message.get("stdout", message.get("stderr", b""))
            if expected in seen:
                return
    raise AssertionError(f"terminal did not emit {expected!r}: {seen[-2000:]!r}")


def resize_workload(socket):
    # The application's trap, not a host-side size query, proves both the
    # dimensions and SIGWINCH crossed every terminal boundary.
    socket.send(
        msgpack.packb({"stdin": b"trap 'printf \"RESIZE-%s\\n\" \"$(stty size)\"' WINCH; printf 'RESIZE-%s\\n' ready\n"}, use_bin_type=True)
    )
    # Canonical projections may end a displayed line with cursor movement,
    # not the raw PTY's CRLF. These expanded markers cannot match the echoed
    # printf command, whose format string still contains the literal %s.
    expect_output(socket, b"RESIZE-ready")
    for rows, cols in ((33, 101), (20, 60), (45, 150)):
        socket.send(msgpack.packb({"control": {"type": "resize", "cols": cols, "rows": rows}}, use_bin_type=True))
        expect_output(socket, f"RESIZE-{rows} {cols}".encode())
    socket.send(msgpack.packb({"stdin": b"trap - WINCH\n"}, use_bin_type=True))


def test_an_image_session_terminal_is_its_workloads_command(service, tmp_path):
    client = service.client()
    with image_session(service, tmp_path, "workload-terminal") as vm_id:
        with terminal(service, vm_id) as socket:
            assert probe(socket) == ("1000", "100000", "workload")
            resize_workload(socket)
            socket.send(msgpack.packb({"stdin": b"echo MARK-BEFORE-DETACH\n"}, use_bin_type=True))
        # Leaving the terminal leaves the workload running; attaching again
        # replays its screen and types into the same shell.
        assert client.get(f"/vms/{vm_id}/container")["state"] == "running"
        with terminal(service, vm_id) as socket:
            assert probe(socket, after=b"MARK-BEFORE-DETACH") == (
                "1000",
                "100000",
                "workload",
            )
            # The command is the workload: when it exits, the workload has.
            socket.send(msgpack.packb({"stdin": b"exit 7\n"}, use_bin_type=True))
            deadline = time.monotonic() + 60
            status = client.get(f"/vms/{vm_id}/container")
            while status["state"] != "exited" and time.monotonic() < deadline:
                time.sleep(0.5)
                status = client.get(f"/vms/{vm_id}/container")
            assert status["state"] == "exited", status
            assert status["exit_code"] == 7, status
