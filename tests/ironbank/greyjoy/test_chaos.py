"""Greyjoy: adversaries against the private network, on real VMs.

Each test breaks one thing while two members talk and checks that only
that thing broke, that it came back, and how long that took. Recovery
times go to the benchmark store like any other measurement, exploratory.
The adversaries: the network's switch killed under traffic, one member's
link pump killed, a connect flood past the private quota, and deleting a
network that still has members.
"""

import contextlib
import json
import os
import signal
import subprocess
import time

import pytest
from helpers.constants import PROJECT_ROOT

from tests.ironbank.kingslanding.test_private_datagram import (
    ECHO_PORT,
    linked,
    member_state,
    udp,
)
from tests.ironbank.kingslanding.test_private_link_benchmark import (
    IN_CONTAINER,
    THROUGHPUT_PORT,
    evidence,
    guest,
    record,
    start_in_guest,
)
from tests.ironbank.kingslanding.test_private_tcp import connect_from, members
from tests.ironbank.kingslanding.test_run import service, wait_for

__all__ = ["evidence", "members", "service"]
pytestmark = pytest.mark.integration

FATAL_LINES = ("virtual machine stopped",)


def owner_logs(service):
    return "\n".join(
        log.read_text(errors="replace") for log in service.tmp_dir.rglob("process.log")
    )


def echo_up(service, alpha, beta):
    return (
        udp(service, alpha, beta["private_address"], 5, 64, wait_ms=500)["received"] > 0
    )


def serve_echo(service, beta):
    start_in_guest(
        service,
        beta["id"],
        "udp-echo",
        f"{IN_CONTAINER} capsem-bench-rs udp --serve 0.0.0.0:{ECHO_PORT}",
    )


def until_echo(service, alpha, beta, what, timeout=90):
    """Seconds until alpha's datagrams reach beta again."""
    started = time.monotonic()
    wait_for(lambda: echo_up(service, alpha, beta), what, timeout=timeout)
    return time.monotonic() - started


def switch_pids():
    out = subprocess.run(
        ["pgrep", "-f", "capsem-router .*--switch"],
        capture_output=True,
        text=True,
        check=False,
    ).stdout
    return [int(pid) for pid in out.split()]


def keep_recovery(evidence, name, seconds):
    record(
        evidence,
        {f"greyjoy.{name}.recovery_seconds": {"unit": "seconds", "samples": [seconds]}},
        subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=PROJECT_ROOT, timeout=5, text=True
        ).strip(),
    )


def test_killing_the_networks_switch_drops_only_that_network_and_members_relink(
    members, service, evidence
):
    alpha, beta = members["alpha"], members["beta"]
    network = members["network"]
    linked(service, network, alpha, beta)
    serve_echo(service, beta)
    until_echo(service, alpha, beta, "the echo answers before the adversary")
    victims = switch_pids()
    assert len(victims) == 1, victims
    os.kill(victims[0], signal.SIGKILL)
    # Only the frame link is gone; the admitted TCP path and both VMs live on.
    start_in_guest(
        service,
        beta["id"],
        "throughput-server",
        f"{IN_CONTAINER} capsem-bench-rs throughput --serve 0.0.0.0:{THROUGHPUT_PORT}",
    )
    target = f"{beta['private_address']}:{THROUGHPUT_PORT}"
    wait_for(
        lambda: connect_from(service, alpha, target)[0].get("exit_code") == 0,
        "private TCP still works with the switch dead",
        timeout=30,
    )
    assert guest(service, alpha["id"], "true")["exit_code"] == 0
    assert guest(service, beta["id"], "true")["exit_code"] == 0
    # The service relinks every member on a fresh switch.
    recovery = until_echo(
        service, alpha, beta, "the echo answers again on a fresh switch"
    )
    assert switch_pids() and switch_pids() != victims
    assert member_state(service, network, beta["id"]) == "ready"
    keep_recovery(evidence, "switch_kill", recovery)
    for line in FATAL_LINES:
        assert line not in owner_logs(service), line


def test_killing_one_members_pump_drops_only_that_link_until_it_restarts(
    members, service, evidence
):
    alpha, beta = members["alpha"], members["beta"]
    network = members["network"]
    linked(service, network, alpha, beta)
    serve_echo(service, beta)
    until_echo(service, alpha, beta, "the echo answers before the adversary")
    killed = guest(
        service, beta["id"], "pkill -9 -x capsem-tun; echo killed", check=False
    )
    assert "killed" in killed.get("stdout", ""), killed
    # The agent restarts the pump, the owner takes the fresh stream, the
    # service links it again: bounded by the supervisor's first backoff and
    # the link handshake.
    wait_for(
        lambda: (
            member_state(service, network, beta["id"]) != "ready"
            or not echo_up(service, alpha, beta)
        ),
        "beta's link is seen to drop",
        timeout=30,
    )
    recovery = until_echo(service, alpha, beta, "beta's link is back", timeout=90)
    assert member_state(service, network, beta["id"]) == "ready"
    # alpha's own link never dropped: the history shows exactly one close for beta.
    logs = service.client().get(f"/networks/{network}/logs")
    closes = [
        event["event"]["network"]["destination"]["vm"]["id"]
        for event in logs.get("events", [])
        if event["event_type"] == "network.close"
        and event["event"]["network"].get("protocol") == "link"
    ]
    assert closes == [beta["id"]], closes
    keep_recovery(evidence, "pump_kill", recovery)


def test_a_connect_flood_past_the_private_quota_is_bounded_and_survived(
    members, service
):
    alpha, beta = members["alpha"], members["beta"]
    linked(service, members["network"], alpha, beta)
    start_in_guest(
        service,
        beta["id"],
        "throughput-server",
        f"{IN_CONTAINER} capsem-bench-rs throughput --serve 0.0.0.0:{THROUGHPUT_PORT}",
    )
    target = f"{beta['private_address']}:{THROUGHPUT_PORT}"
    wait_for(
        lambda: connect_from(service, alpha, target)[0].get("exit_code") == 0,
        "alpha reaches beta",
        timeout=60,
    )
    # Three clients of 32 streams at once: past the destination's private
    # quota of 64. Some are refused; nothing dies, and every stream that was
    # admitted is closed afterwards rather than kept for the VM's life.
    for burst in range(3):
        start_in_guest(
            service,
            alpha["id"],
            f"flood-{burst}",
            f"{IN_CONTAINER} capsem-bench-rs throughput --direction latency --streams 32 --seconds 3 --address {target}",
        )
    time.sleep(6)
    assert guest(service, alpha["id"], "true")["exit_code"] == 0
    assert guest(service, beta["id"], "true")["exit_code"] == 0
    for line in FATAL_LINES:
        assert line not in owner_logs(service), line
    after, elapsed = connect_from(service, alpha, target)
    assert after.get("exit_code") == 0, after
    assert elapsed < 10, elapsed

    def open_private_flows():
        log = "".join(
            log.read_text(errors="replace")
            for log in service.tmp_dir.glob(f"persistent/{beta['id']}/process.log")
        )
        return log.count('"publication connection accepted"') - log.count(
            '"publication closed"'
        )

    wait_for(
        lambda: open_private_flows() <= 1,
        "every admitted flow of the flood is closed again",
        timeout=30,
    )


def test_deleting_a_network_with_members_is_refused_and_flows_go_on(members, service):
    alpha, beta = members["alpha"], members["beta"]
    network = members["network"]
    linked(service, network, alpha, beta)
    serve_echo(service, beta)
    until_echo(service, alpha, beta, "the echo answers")
    with contextlib.suppress(Exception):
        service.client().delete(f"/networks/{network}")
    inspected = service.client().get(f"/networks/{network}")
    assert len(inspected["members"]) == 2, inspected
    assert echo_up(service, alpha, beta)
    assert json.dumps(inspected)
