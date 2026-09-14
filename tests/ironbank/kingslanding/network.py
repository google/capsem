"""Members of named networks, as the switch tests see them from outside.

Every VM starts unplugged. Joining a network leases the member an address in
that network's subnet and plugs one cable -- a tap named `cable<N>` in the
guest -- into the network's switch, one `capsem-router --network` process per
network. Everything these helpers read is public: `GET /networks/{id}` for
each member's address and state, the guest's own view of its cables, and the
host's view of the switch processes and the descriptors they hold.
"""

import contextlib
import ctypes
import ipaddress
import json
import os
import re
import shlex
import subprocess
import sys

import pytest

from tests.fixtures.oci.registry import registry
from tests.ironbank.kingslanding.test_publish_benchmark import (
    IN_CONTAINER,
    THROUGHPUT_PORT,
    guest,
    start_in_guest,
)
from tests.ironbank.kingslanding.test_run import created, wait_for

ECHO_PORT = 5202
# Cables declare 10 Gb/s with the largest frame a u16 record carries.
CABLE_SPEED_MBPS = "10000"
CABLE_MTU = str(65535 - 14)
KEPT_LOGS = ("process.log", "serial.log", ".capsem-agent-stdio.log")
# The container's own resolver, for a probe that runs from the guest's mount
# namespace inside the container's network namespace: the guest's resolv.conf
# names a loopback proxy the container cannot reach, while the container
# resolves through its gateway (launch.py).
AS_CONTAINER = (
    "printf 'nameserver 10.0.1.1\\n' > /var/tmp/container-resolv.conf && "
    "unshare -m sh -c 'mount --bind /var/tmp/container-resolv.conf /etc/resolv.conf"
    ' && exec nsenter -t "$(cat /var/tmp/capsem-container/workload.pid)" -n "$@"\' sh'
)


def network_info(service, network):
    return service.client().get(f"/networks/{network}")


def membership(service, network, vm_id):
    for member in network_info(service, network)["members"]:
        if member["vm_id"] == vm_id:
            return member
    return None


def member_state(service, network, vm_id):
    member = membership(service, network, vm_id)
    return member["state"] if member else "absent"


def join(service, network, vm):
    status, joined = service.client().call_json("PUT", f"/networks/{network}/members/{vm['id']}")
    assert status == 200, (status, joined)


def leave(service, network, vm):
    status, left = service.client().call_json("DELETE", f"/networks/{network}/members/{vm['id']}")
    assert status == 200, (status, left)


def linked(service, network, *vms, timeout=90):
    wait_for(
        lambda: all(member_state(service, network, vm["id"]) == "ready" for vm in vms),
        "every member's cable is plugged into the network's switch",
        timeout=timeout,
    )


def address_of(service, network, vm):
    member = membership(service, network, vm["id"])
    assert member is not None, (network, vm["id"])
    return member["address"]


def cables(service, vm):
    """The guest's cables: device name -> `address/prefix`."""
    shown = guest(service, vm["id"], "ip -o -4 addr show")["stdout"]
    return {
        fields[1]: fields[3]
        for fields in (line.split() for line in shown.splitlines())
        if fields[1].startswith("cable")
    }


def cable_of(service, vm, address):
    for device, cidr in cables(service, vm).items():
        if cidr.split("/")[0] == address:
            return device
    raise AssertionError(f"{vm['id']} has no cable holding {address}")


def mac_of(address):
    """The MAC a member's cable carries: locally administered, its address."""
    return "02:ca:" + ":".join(f"{octet:02x}" for octet in ipaddress.IPv4Address(address).packed)


def stranger_in(subnet):
    """An address inside `subnet` that no test ever leases."""
    return str(ipaddress.IPv4Network(subnet)[200])


def switch_pids(service):
    """This service's switch processes: its own children, so parallel test
    workers' switches are not counted."""
    listed = subprocess.run(
        ["pgrep", "-P", str(service.proc.pid), "-f", r"capsem-router .*--network"],
        capture_output=True,
        text=True,
        check=False,
    ).stdout
    return sorted(int(pid) for pid in listed.split())


def socket_descriptors(pid):
    """How many sockets `pid` holds, and the listing that says so."""
    if sys.platform == "linux":
        root = f"/proc/{pid}/fd"
        links = {fd: os.readlink(f"{root}/{fd}") for fd in os.listdir(root)}
        sockets = {fd: link for fd, link in links.items() if link.startswith("socket:")}
        return len(sockets), json.dumps(links, indent=2)
    listing = subprocess.run(
        ["lsof", "-a", "-n", "-P", "-p", str(pid), "-U"],
        capture_output=True,
        text=True,
        timeout=15,
        check=False,
    ).stdout
    return sum(1 for line in listing.splitlines()[1:] if line.strip()), listing


class _TaskInfo(ctypes.Structure):
    """libproc's `struct proc_taskinfo` (PROC_PIDTASKINFO)."""

    _fields_ = [
        (name, ctypes.c_uint64)
        for name in ("virtual_size", "resident_size", "total_user", "total_system", "threads_user", "threads_system")
    ] + [
        (name, ctypes.c_int32)
        for name in (
            "policy", "faults", "pageins", "cow_faults", "messages_sent", "messages_received",
            "syscalls_mach", "syscalls_unix", "csw", "threadnum", "numrunning", "priority",
        )
    ]


_PROC_PIDTASKINFO = 4


def resident_kib(pid):
    """`pid`'s resident memory, by syscall: the gate runs this suite under
    Seatbelt, where setuid `ps` cannot execute."""
    if sys.platform == "linux":
        with open(f"/proc/{pid}/status") as status:
            for line in status:
                if line.startswith("VmRSS:"):
                    return int(line.split()[1])
        raise AssertionError(f"no VmRSS for {pid}")
    info = _TaskInfo()
    libproc = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
    size = libproc.proc_pidinfo(pid, _PROC_PIDTASKINFO, ctypes.c_uint64(0), ctypes.byref(info), ctypes.sizeof(info))
    assert size == ctypes.sizeof(info), (pid, size, ctypes.get_errno())
    return info.resident_size // 1024


def network_events(service, network):
    return service.client().get(f"/networks/{network}/logs").get("events", [])


def cable_rows(service, network, event_type, vm_id):
    """The network's audit rows of `event_type` about `vm_id`'s cable."""
    return [
        event
        for event in network_events(service, network)
        if event["event_type"] == event_type
        and event["event"]["network"]["protocol"] == "link"
        and event["event"]["network"]["source"]["vm"]["id"] == vm_id
    ]


def bench_in(service, vm, *args, timeout=40, by_name=False, check=True):
    """One `capsem-bench-rs` run inside `vm`'s container; the exec response."""
    result = guest(
        service,
        vm["id"],
        f"{AS_CONTAINER if by_name else IN_CONTAINER} "
        + shlex.join(["capsem-bench-rs", *args]),
        timeout=timeout,
        check=False,
    )
    if check:
        assert result.get("exit_code") == 0, result
    return result


def report_of(result, recorded=None, lane=None):
    """A bench JSON report's counts; with `recorded`, every metric joins the
    store under `lane`."""
    metrics = json.loads(result["stdout"])["metrics"]
    if recorded is not None:
        for name, metric in metrics.items():
            recorded.setdefault(f"{lane}.{name}", {"unit": metric["unit"], "samples": []})[
                "samples"
            ].extend(metric["samples"])
    return {key: metrics[key]["samples"][0] for key in ("sent", "received", "lost")} | {
        "round_trips": metrics.get("round_trip_ms", {}).get("samples", [])
    }


def udp(service, vm, address, count, size, interval_ms=2, wait_ms=2000, recorded=None, lane=None, by_name=False):
    result = bench_in(
        service,
        vm,
        "udp",
        "--address",
        f"{address}:{ECHO_PORT}",
        "--count",
        str(count),
        "--size",
        str(size),
        "--interval-ms",
        str(interval_ms),
        "--wait-ms",
        str(wait_ms),
        by_name=by_name,
    )
    return report_of(result, recorded, lane)


def ping(service, vm, address, count, interval_ms=20, timeout_ms=None, recorded=None, lane=None, by_name=False):
    args = ["ping", "--address", address, "--count", str(count), "--interval-ms", str(interval_ms)]
    if timeout_ms is not None:
        args += ["--timeout-ms", str(timeout_ms)]
    return report_of(bench_in(service, vm, *args, by_name=by_name), recorded, lane)


def serve_echo(service, vm):
    start_in_guest(
        service,
        vm["id"],
        "udp-echo",
        f"{IN_CONTAINER} capsem-bench-rs udp --serve 0.0.0.0:{ECHO_PORT}",
    )


def echo_answers(service, vm, address):
    return udp(service, vm, address, 5, 64, wait_ms=500)["received"] > 0


def keep_logs(service, evidence):
    """Every VM's owner log, console and agent log, where the gate cannot
    delete them."""
    for name in KEPT_LOGS:
        for log in service.tmp_dir.rglob(name):
            stem = "-".join(log.relative_to(service.tmp_dir).parts[:-1])
            (evidence / f"{stem}-{name}").write_bytes(log.read_bytes())


@pytest.fixture
def members(service, tmp_path, evidence):
    """Two Redis container VMs plugged into one network, `team`: `alpha` with a
    published throughput port, `beta` without. Also yields the image, so a test
    can boot a third VM from the same registry."""
    with (
        registry(tmp_path) as (reference, certificate, _),
        created(service, tmp_path, reference, certificate, "alpha", "-p", f"0:{THROUGHPUT_PORT}") as alpha,
        created(service, tmp_path, reference, certificate, "beta") as beta,
    ):
        client = service.client()
        network = client.post("/networks", {"name": "team"})
        try:
            for vm in (alpha, beta):
                join(service, network["id"], vm)
            linked(service, network["id"], alpha, beta)
            yield {
                "alpha": alpha,
                "beta": beta,
                "network": network["id"],
                "subnet": network["subnet"],
                "image": (reference, certificate),
                "tmp_path": tmp_path,
            }
        finally:
            keep_logs(service, evidence)
            for row in client.get("/vms/list")["sandboxes"]:
                with contextlib.suppress(Exception):
                    client.delete(f"/vms/{row['id']}/delete")


@contextlib.contextmanager
def another(service, members, name):
    """One more container VM from the members' image, deleted afterwards."""
    reference, certificate = members["image"]
    with created(service, members["tmp_path"], reference, certificate, name) as vm:
        yield vm


def published_port(vm):
    mappings = re.findall(
        rf"Published 127.0.0.1:(\d+) -> {THROUGHPUT_PORT}/tcp", vm["stderr"].read_text()
    )
    assert len(mappings) == 1, vm["stderr"].read_text()
    return int(mappings[0])
