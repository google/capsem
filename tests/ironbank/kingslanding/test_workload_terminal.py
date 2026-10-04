"""An image session's terminal is a shell in its workload, never in the VM.

`capsem create --image` marks the session (capsem-core `WORKLOAD_ENV`), so the
VM's terminal shell hands its PTY to the launcher's attach loop before any VM
prompt. What the user types then runs through `runc exec` as the image's own
user, inside the workload's user namespace; ending that shell enters the
workload again instead of exposing the VM shell underneath.
"""

import json
import re
import time

import pytest
from websockets.sync.client import connect
from websockets.typing import Subprotocol

from tests.fixtures.oci.registry import registry
from tests.ironbank.kingslanding.test_run import created, service
from tests.ironbank.kingslanding.test_workload_exec import BUNDLE_USER, run

__all__ = ["service"]

pytestmark = pytest.mark.integration

STDIN, STDOUT, STDERR, CONTROL = 0, 1, 2, 3
# Typed into the terminal. The echoed line holds `$(id -u)` unexpanded, so only
# the shell's own output matches PROBE.
TYPED = 'read _ lower _ < /proc/self/uid_map; echo "PROBE-$(id -u)-$lower-$([ -d /var/tmp/capsem-container ] && echo vm || echo workload)"\n'
PROBE = re.compile(r"PROBE-(\d+)-(\d+)-(vm|workload)")


def terminal(service, vm_id):
    port = int((service.tmp_dir / "gateway.port").read_text())
    token = (service.tmp_dir / "gateway.token").read_text().strip()
    socket = connect(
        f"ws://127.0.0.1:{port}/vms/{vm_id}/stream",
        subprotocols=[Subprotocol("capsem.stream.v1")],
        additional_headers={"Authorization": f"Bearer {token}"},
        open_timeout=10,
    )
    socket.send(bytes([CONTROL]) + json.dumps({"type": "start", "kind": "terminal"}).encode())
    return socket


def probe(socket, after=b"", deadline=90):
    """Type the probe until the terminal answers it after `after`; the first
    attempts may land while the attach loop is still waiting for the workload."""
    seen = b""
    end = time.monotonic() + deadline
    next_send = 0.0
    while time.monotonic() < end:
        if time.monotonic() >= next_send:
            socket.send(bytes([STDIN]) + TYPED.encode())
            next_send = time.monotonic() + 5
        try:
            frame = socket.recv(timeout=1)
        except TimeoutError:
            continue
        if isinstance(frame, bytes) and frame[:1] in (bytes([STDOUT]), bytes([STDERR])):
            seen += frame[1:]
            start = seen.find(after)
            found = start >= 0 and PROBE.search(seen[start + len(after) :].decode(errors="replace"))
            if found:
                return found.groups()
    raise AssertionError(f"the terminal never answered the probe: {seen[-2000:]!r}")


def test_an_image_session_terminal_is_a_workload_shell_and_stays_one(service, tmp_path):
    client = service.client()
    with (
        registry(tmp_path) as (reference, certificate, _),
        created(service, tmp_path, reference, certificate, "workload-terminal") as vm,
    ):
        uid, _ = run(client, vm["id"], BUNDLE_USER, target="vm").split()
        with terminal(service, vm["id"]) as socket:
            assert probe(socket) == (uid, "100000", "workload")
            # Ending the workload shell enters the workload again: the VM shell
            # under the attach loop is never handed to the terminal.
            socket.send(bytes([STDIN]) + b"exit\n")
            assert probe(socket, after=b"entering it again") == (uid, "100000", "workload")
