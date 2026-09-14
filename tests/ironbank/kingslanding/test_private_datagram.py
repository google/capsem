"""UDP and ICMP between two real members of a named network.

Two containers boot in two VMs and join one network. Each guest's tap0 is
one ethernet link to the network's confined switch, and the container
reaches it through the guest's NAT: datagrams from the first container
reach an echo server in the second on its private address and come back,
and echo requests are answered by the second VM's kernel. Nothing on the
host parses past the frame's addresses. A stranger's address gets nothing,
leaving the network ends it at once, joining again brings it back, and a
burst never stops a VM. The network's history names each link.
"""

import json
import shlex
import subprocess

import pytest
from helpers.constants import PROJECT_ROOT

from tests.ironbank.kingslanding.test_private_link_benchmark import (
    IN_CONTAINER,
    evidence,
    guest,
    record,
    start_in_guest,
)
from tests.ironbank.kingslanding.test_private_tcp import STRANGER, members
from tests.ironbank.kingslanding.test_run import service, wait_for

__all__ = ["evidence", "members", "service"]
pytestmark = pytest.mark.integration

ECHO_PORT = 5202
# The container's own resolver, for a probe that runs from the guest's
# mount namespace inside the container's network namespace: the guest's
# resolv.conf names a loopback proxy the container's namespace cannot
# reach, while the container resolves through its gateway (launch.py).
AS_CONTAINER = (
    "printf 'nameserver 10.0.1.1\\n' > /var/tmp/container-resolv.conf && "
    "unshare -m sh -c 'mount --bind /var/tmp/container-resolv.conf /etc/resolv.conf"
    ' && exec nsenter -t "$(cat /var/tmp/capsem-container/workload.pid)" -n "$@"\' sh'
)
# What the switch never lets through, and what the owner logs when the VM
# is gone from under it: none of it may appear during a burst.
FATAL_LINES = ("virtual machine stopped",)


def member_state(service, network, vm_id):
    for member in service.client().get(f"/networks/{network}")["members"]:
        if member["vm_id"] == vm_id:
            return member["state"]
    return "absent"


def linked(service, network, *vms):
    wait_for(
        lambda: all(member_state(service, network, vm["id"]) == "ready" for vm in vms),
        "every member is linked to the network's switch",
        timeout=90,
    )


def probe_fails(service, vm, *args, timeout=40):
    """Run one bench probe inside `vm`'s container, with its resolver,
    expecting it to fail before measuring (an unresolvable name); its output."""
    result = guest(
        service,
        vm["id"],
        f"{AS_CONTAINER} " + shlex.join(["capsem-bench-rs", *args]),
        timeout=timeout,
        check=False,
    )
    assert result.get("exit_code") not in (None, 0), result
    return result.get("stdout", "") + result.get("stderr", "")


def probe(service, vm, *args, timeout=40, recorded=None, lane=None, by_name=False):
    """Run one bench probe inside `vm`'s container; its JSON report. With
    `recorded` and `lane`, every metric joins the store under that lane;
    `by_name` gives the probe the container's resolver."""
    result = guest(
        service,
        vm["id"],
        f"{AS_CONTAINER if by_name else IN_CONTAINER} "
        + shlex.join(["capsem-bench-rs", *args]),
        timeout=timeout,
        check=False,
    )
    assert result.get("exit_code") == 0, result
    report = json.loads(result["stdout"])
    metrics = report["metrics"]
    if recorded is not None:
        for name, metric in metrics.items():
            recorded.setdefault(
                f"{lane}.{name}", {"unit": metric["unit"], "samples": []}
            )["samples"].extend(metric["samples"])
    return {key: metrics[key]["samples"][0] for key in ("sent", "received", "lost")} | {
        "round_trips": metrics.get("round_trip_ms", {}).get("samples", [])
    }


def udp(
    service,
    vm,
    address,
    count,
    size,
    interval_ms=2,
    wait_ms=2000,
    recorded=None,
    lane=None,
    by_name=False,
):
    return probe(
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
        recorded=recorded,
        lane=lane,
        by_name=by_name,
    )


ICMP, IPIP, GRE = 1, 4, 47
# Raw sockets in beta's own namespace: the guest has no packet sockets, and
# nothing filters its input on tap0. ICMP is the control -- the switch
# forwards echo and nothing redirects it -- so a tunnel protocol the switch
# let through would be heard the same way.
RAW_LISTENER = """
import json, select, socket, sys, time
source, protocols = sys.argv[1], [int(p) for p in sys.argv[2:]]
sockets = {socket.socket(socket.AF_INET, socket.SOCK_RAW, p): p for p in protocols}
heard = {p: 0 for p in protocols}
open("/var/tmp/raw-listener.ready", "w").close()
deadline = time.monotonic() + 12
while time.monotonic() < deadline:
    for s in select.select(list(sockets), [], [], 0.2)[0]:
        if s.recvfrom(65535)[1][0] == source:
            heard[sockets[s]] += 1
json.dump(heard, open("/var/tmp/raw-listener.json", "w"))
"""
# An echo request header, so the ICMP control is a type the switch forwards.
RAW_SENDER = """
import socket, sys
destination, protocols = sys.argv[1], [int(p) for p in sys.argv[2:]]
for p in protocols:
    s = socket.socket(socket.AF_INET, socket.SOCK_RAW, p)
    for _ in range(10):
        s.sendto(b"\\x08\\x00\\x00\\x00\\x00\\x01\\x00\\x01capsem!!", (destination, 0))
"""


def tunnel_probe(service, alpha, beta):
    """What beta's kernel receives from alpha for ICMP, GRE and IPIP."""
    protocols = " ".join(str(p) for p in (ICMP, GRE, IPIP))
    guest(
        service,
        beta["id"],
        "rm -f /var/tmp/raw-listener.ready /var/tmp/raw-listener.json",
    )
    start_in_guest(
        service,
        beta["id"],
        "raw-listener",
        f"python3 -c {shlex.quote(RAW_LISTENER)} {alpha['private_address']} {protocols}",
    )

    def exists(path):
        return (
            guest(service, beta["id"], f"test -e {path}", check=False).get("exit_code")
            == 0
        )

    def listener_log():
        return guest(
            service, beta["id"], "cat /var/tmp/raw-listener.log", check=False
        ).get("stdout", "")

    try:
        wait_for(
            lambda: exists("/var/tmp/raw-listener.ready"),
            "beta listens on raw sockets",
            timeout=20,
        )
        guest(
            service,
            alpha["id"],
            f"python3 -c {shlex.quote(RAW_SENDER)} {beta['private_address']} {protocols}",
        )
        wait_for(
            lambda: exists("/var/tmp/raw-listener.json"),
            "beta's listener reports",
            timeout=30,
        )
    except AssertionError as error:
        raise AssertionError(f"{error}; listener log: {listener_log()!r}") from None
    return json.loads(
        guest(service, beta["id"], "cat /var/tmp/raw-listener.json")["stdout"]
    )


def owner_logs(service):
    return "\n".join(log.read_text() for log in service.tmp_dir.rglob("process.log"))


def test_members_exchange_udp_and_icmp_over_the_link_and_strangers_get_nothing(
    members, service, evidence
):
    alpha, beta = members["alpha"], members["beta"]
    network = members["network"]
    recorded = {}
    linked(service, network, alpha, beta)
    start_in_guest(
        service,
        beta["id"],
        "udp-echo",
        f"{IN_CONTAINER} capsem-bench-rs udp --serve 0.0.0.0:{ECHO_PORT}",
    )
    wait_for(
        lambda: (
            udp(service, alpha, beta["private_address"], 5, 64, wait_ms=500)["received"]
            > 0
        ),
        "alpha's datagrams reach beta's echo server",
        timeout=60,
    )

    # Ordinary datagrams, a frame-sized one, and one the guest fragments;
    # each lane's counts and round trips go to the benchmark store.
    lanes = {"b1400": (200, 1400), "b60000": (20, 60_000), "b65507": (10, 65_507)}
    results = {
        lane: udp(
            service,
            alpha,
            beta["private_address"],
            count,
            size,
            recorded=recorded,
            lane=f"private_udp.{lane}",
        )
        for lane, (count, size) in lanes.items()
    }
    small = results["b1400"]
    assert small["received"] >= 195, small
    assert small["round_trips"] and max(small["round_trips"]) < 500, small
    assert results["b60000"]["received"] >= 19, results
    assert results["b65507"]["received"] >= 9, results

    # The second VM's kernel answers echo requests on the link.
    ping = probe(
        service,
        alpha,
        "ping",
        "--address",
        beta["private_address"],
        "--count",
        "20",
        "--interval-ms",
        "20",
        recorded=recorded,
        lane="private_ping",
    )
    assert ping["received"] >= 19, ping
    source_commit = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=PROJECT_ROOT, timeout=5, text=True
    ).strip()
    record(evidence, recorded, source_commit)
    assert (evidence / "report.txt").exists()

    # Members have names on the private zone, answered by the host for the
    # asker's networks only; a stranger's name does not resolve at all.
    named = udp(service, alpha, "beta.team.capsem.internal", 20, 64, by_name=True)
    assert named["received"] >= 19, named
    short = udp(service, alpha, "beta.capsem.internal", 5, 64, by_name=True)
    assert short["received"] >= 4, short
    by_name = probe(
        service,
        alpha,
        "ping",
        "--address",
        "beta.team.capsem.internal",
        "--count",
        "3",
        by_name=True,
    )
    assert by_name["received"] >= 2, by_name
    unresolved = probe_fails(
        service,
        alpha,
        "udp",
        "--address",
        f"stranger.team.capsem.internal:{ECHO_PORT}",
        "--count",
        "1",
    )
    assert "resolve" in unresolved, unresolved
    lookup = guest(
        service,
        alpha["id"],
        f"{AS_CONTAINER} getent hosts beta.team.capsem.internal; "
        f"{AS_CONTAINER} getent hosts {beta['private_address']}",
        check=False,
    )
    assert beta["private_address"] in lookup.get("stdout", ""), lookup
    assert "beta.team.capsem.internal" in lookup.get("stdout", ""), lookup

    # A private address that belongs to no member gets nothing, in bound.
    stranger = udp(service, alpha, STRANGER, 5, 64, wait_ms=1000)
    assert stranger["received"] == 0, stranger
    none = probe(
        service,
        alpha,
        "ping",
        "--address",
        STRANGER,
        "--count",
        "3",
        "--timeout-ms",
        "500",
    )
    assert none["received"] == 0, none

    # Only UDP and ICMP cross: a tunnel protocol would carry TCP around its
    # admission. Beta's kernel hands raw sockets every packet of a protocol,
    # so what the switch forwards is what beta hears; ICMP is the control.
    tunnels = tunnel_probe(service, alpha, beta)
    assert tunnels[str(ICMP)] >= 9, tunnels
    assert tunnels[str(GRE)] == 0 and tunnels[str(IPIP)] == 0, tunnels

    # The history names both links.
    def audited():
        logs = service.client().get(f"/networks/{network}/logs")
        seen = {
            event["event"]["network"]["destination"]["vm"]["id"]
            for event in logs.get("events", [])
            if event["event"]["network"].get("protocol") == "link"
            and event["event"]["decision"]["effective"] == "allow"
        }
        return {alpha["id"], beta["id"]} <= seen

    wait_for(audited, "network history records both links", timeout=20)


def test_leaving_ends_the_link_and_joining_again_restores_it(members, service):
    alpha, beta = members["alpha"], members["beta"]
    network = members["network"]
    linked(service, network, alpha, beta)
    start_in_guest(
        service,
        beta["id"],
        "udp-echo",
        f"{IN_CONTAINER} capsem-bench-rs udp --serve 0.0.0.0:{ECHO_PORT}",
    )
    wait_for(
        lambda: (
            udp(service, alpha, beta["private_address"], 5, 64, wait_ms=500)["received"]
            > 0
        ),
        "alpha's datagrams reach beta's echo server",
        timeout=60,
    )
    client = service.client()
    client.delete(f"/networks/{network}/members/{beta['id']}")
    gone = udp(service, alpha, beta["private_address"], 5, 64, wait_ms=1000)
    assert gone["received"] == 0, gone
    # The name went with the membership.
    probe_fails(
        service,
        alpha,
        "udp",
        "--address",
        f"beta.team.capsem.internal:{ECHO_PORT}",
        "--count",
        "1",
    )

    # Joining again: the pump reconnected, the owner holds a fresh stream,
    # and the switch takes it.
    client.put(f"/networks/{network}/members/{beta['id']}")
    linked(service, network, beta)
    wait_for(
        lambda: (
            udp(service, alpha, beta["private_address"], 5, 64, wait_ms=500)["received"]
            > 0
        ),
        "the relinked member answers again",
        timeout=60,
    )

    # A burst past the switch's queues loses frames, never a VM.
    burst = udp(
        service,
        alpha,
        beta["private_address"],
        5000,
        1400,
        interval_ms=0,
        wait_ms=3000,
    )
    assert burst["received"] > 0, burst
    assert guest(service, beta["id"], "true")["exit_code"] == 0
    assert guest(service, alpha["id"], "true")["exit_code"] == 0
    logs = owner_logs(service)
    for line in FATAL_LINES:
        assert line not in logs, f"an owner logged {line!r}"
