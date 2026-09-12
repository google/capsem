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

import pytest

from tests.ironbank.kingslanding.test_private_link_benchmark import (
    IN_CONTAINER,
    evidence,
    guest,
    start_in_guest,
)
from tests.ironbank.kingslanding.test_private_tcp import STRANGER, members
from tests.ironbank.kingslanding.test_run import service, wait_for

__all__ = ["evidence", "members", "service"]
pytestmark = pytest.mark.integration

ECHO_PORT = 5202
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


def probe(service, vm, *args, timeout=40):
    """Run one bench probe inside `vm`'s container; its JSON report."""
    result = guest(
        service,
        vm["id"],
        f"{IN_CONTAINER} " + shlex.join(["capsem-bench-rs", *args]),
        timeout=timeout,
        check=False,
    )
    assert result.get("exit_code") == 0, result
    report = json.loads(result["stdout"])
    metrics = report["metrics"]
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

    # Ordinary datagrams, a frame-sized one, and one the guest fragments.
    small = udp(service, alpha, beta["private_address"], 200, 1400)
    assert small["received"] >= 195, small
    assert small["round_trips"] and max(small["round_trips"]) < 500, small
    large = udp(service, alpha, beta["private_address"], 20, 60_000)
    assert large["received"] >= 19, large
    fragmented = udp(service, alpha, beta["private_address"], 10, 65_507)
    assert fragmented["received"] >= 9, fragmented

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
    )
    assert ping["received"] >= 19, ping

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
