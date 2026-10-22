"""A VM cannot take over another VM's KVM host control listener."""

import re
import sys

import pytest

from tests.ironbank.kingslanding.network import members
from tests.ironbank.kingslanding.test_publish_benchmark import evidence, guest
from tests.ironbank.kingslanding.test_run import service

__all__ = ["evidence", "members", "service"]
pytestmark = [pytest.mark.integration, pytest.mark.skipif(sys.platform != "linux", reason="KVM vsock is Linux-only")]


def test_foreign_vm_cannot_hold_its_siblings_control_vsock(members, service):
    alpha, beta = members["alpha"], members["beta"]
    marker = "/root/capsem-peer-vsock-marker"
    assert guest(service, beta["id"], f"printf VM_B_ONLY > {marker}")["exit_code"] == 0
    assert guest(service, alpha["id"], f"test ! -e {marker}")["exit_code"] == 0
    assert guest(service, beta["id"], f"cat {marker}")["stdout"].strip() == "VM_B_ONLY"

    def offset(vm):
        cmdline = guest(service, vm["id"], "cat /proc/cmdline")["stdout"]
        match = re.search(r"(?:^| )capsem\.vsock_port_offset=(\d+)(?: |$)", cmdline)
        assert match is not None, cmdline
        return int(match.group(1))

    alpha_offset, beta_offset = offset(alpha), offset(beta)
    assert alpha_offset != beta_offset, "the two running VMs need distinct vsock port blocks"

    # Host CID 2 and beta's physical control port are both visible to an
    # untrusted guest. A vulnerable listener accepts alpha's CID and parks
    # on the first Ready frame; the fixed listener promptly closes the peer.
    script = f"""
import socket
import sys

peer = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM)
peer.settimeout(1.5)
try:
    peer.connect((2, {beta_offset} + 5000))
    try:
        data = peer.recv(1)
    except (ConnectionResetError, BrokenPipeError):
        pass
    except TimeoutError:
        sys.exit("foreign VM control connection stayed open")
    else:
        if data:
            sys.exit("foreign VM control connection returned host data")
except (ConnectionRefusedError, ConnectionResetError, BrokenPipeError):
    pass
finally:
    peer.close()
"""
    probe = guest(service, alpha["id"], f"python3 - <<'PY'\n{script}\nPY", timeout=15, check=False)
    assert probe.get("exit_code") == 0, probe
    assert guest(service, alpha["id"], "true")["exit_code"] == 0
    assert guest(service, beta["id"], "true")["exit_code"] == 0
