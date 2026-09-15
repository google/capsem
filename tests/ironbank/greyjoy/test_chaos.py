"""Greyjoy: adversaries against private networks, on real VMs.

Each test breaks one thing while members talk and checks that only that
thing broke, that it came back, and how long that took. Recovery times go to
the benchmark store like any other measurement, exploratory. The adversaries:
one network's switch killed while a VM is on two networks, one member's
cable pump killed, a member speaking as another address and flooding
broadcasts, a member plugged and unplugged over and over, and a network
deleted while it still has members.
"""

import ipaddress
import os
import shlex
import signal
import subprocess
import time

import pytest
from helpers.constants import PROJECT_ROOT

from tests.ironbank.kingslanding.network import (
    ECHO_PORT,
    address_of,
    another,
    cable_of,
    cable_rows,
    cables,
    echo_answers,
    join,
    leave,
    linked,
    member_state,
    members,
    network_events,
    network_info,
    resident_kib,
    serve_echo,
    socket_descriptors,
    stranger_in,
    switch_pids,
    udp,
)
from tests.ironbank.kingslanding.test_publish_benchmark import (
    evidence,
    guest,
    record,
    start_in_guest,
)
from tests.ironbank.kingslanding.test_run import service, wait_for

__all__ = ["evidence", "members", "service"]
pytestmark = pytest.mark.integration

FATAL_LINES = ("virtual machine stopped",)
# What one switch may grow by under a flood: two ports' queues of 64 full
# frames each, and their read buffers, with room to spare.
FLOOD_GROWTH_KIB = 32 * 1024
CHURN_ROUNDS = 20
# Five datagrams to the peer's echo from a chosen local address, in the
# guest's own namespace; prints how many were answered.
FROM_ADDRESS = """
import socket, sys
source, destination, port = sys.argv[1], sys.argv[2], int(sys.argv[3])
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
s.bind((source, 0))
s.settimeout(2)
answered = 0
for _ in range(5):
    s.sendto(b"capsem", (destination, port))
    try:
        s.recv(64)
        answered += 1
    except socket.timeout:
        pass
print(answered)
"""
# Broadcasts on the network's subnet for `seconds`, as fast as the guest sends.
BROADCAST_FLOOD = """
import socket, sys, time
broadcast, seconds = sys.argv[1], float(sys.argv[2])
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
s.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
deadline, sent = time.monotonic() + seconds, 0
while time.monotonic() < deadline:
    try:
        s.sendto(b"x" * 512, (broadcast, 9))
        sent += 1
    except OSError:
        pass
print(sent)
"""


def owner_logs(service):
    return "\n".join(log.read_text(errors="replace") for log in service.tmp_dir.rglob("process.log"))


def until_echo(service, vm, address, what, timeout=90):
    """Seconds until `vm`'s datagrams reach the echo at `address` again."""
    started = time.monotonic()
    wait_for(lambda: echo_answers(service, vm, address), what, timeout=timeout)
    return time.monotonic() - started


def keep_recovery(evidence, name, seconds):
    record(
        evidence,
        {f"greyjoy.{name}.recovery_seconds": {"unit": "seconds", "samples": [seconds]}},
        subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=PROJECT_ROOT, timeout=5, text=True).strip(),
    )


def answered_from(service, vm, source, destination):
    script = shlex.quote(FROM_ADDRESS)
    result = guest(service, vm["id"], f"python3 -c {script} {source} {destination} {ECHO_PORT}", timeout=30)
    return int(result["stdout"].strip())


def test_killing_one_networks_switch_leaves_the_other_network_and_restores_only_members(
    members, service, evidence
):
    alpha, beta, team = members["alpha"], members["beta"], members["network"]
    with another(service, members, "gamma") as gamma:
        # Gamma was on team and left it before the adversary: recovery must
        # not plug it back in.
        join(service, team, gamma)
        linked(service, team, gamma)
        gamma_on_team = address_of(service, team, gamma)
        leave(service, team, gamma)
        wait_for(lambda: len(cables(service, gamma)) == 0, "gamma's team cable is down", timeout=20)
        (team_switch,) = switch_pids(service)
        other = service.client().post("/networks", {"name": "other"})
        for vm in (alpha, gamma):
            join(service, other["id"], vm)
        linked(service, other["id"], alpha, gamma)
        (other_switch,) = set(switch_pids(service)) - {team_switch}
        beta_address, gamma_address = address_of(service, team, beta), address_of(service, other["id"], gamma)
        serve_echo(service, beta)
        serve_echo(service, gamma)
        until_echo(service, alpha, beta_address, "team answers before the adversary")
        until_echo(service, alpha, gamma_address, "other answers before the adversary")

        os.kill(team_switch, signal.SIGKILL)
        # The other network never notices: same switch, answering at once.
        assert echo_answers(service, alpha, gamma_address)
        assert other_switch in switch_pids(service)
        recovery = until_echo(service, alpha, beta_address, "team answers again on a fresh switch")
        assert team_switch not in switch_pids(service) and other_switch in switch_pids(service)
        assert len(switch_pids(service)) == 2
        wait_for(
            lambda: member_state(service, team, beta["id"]) == "ready",
            "beta's membership is ready again",
            timeout=30,
        )
        # Only members came back: gamma is not on team, has no team cable,
        # and its old team address answers nobody.
        assert {member["vm_id"] for member in network_info(service, team)["members"]} == {alpha["id"], beta["id"]}
        assert list(cables(service, gamma).values()) == [f"{gamma_address}/{other['subnet'].split('/')[1]}"]
        assert udp(service, beta, gamma_on_team, 5, 64, wait_ms=1000)["received"] == 0
        assert guest(service, alpha["id"], "true")["exit_code"] == 0
        keep_recovery(evidence, "switch_kill", recovery)
        for line in FATAL_LINES:
            assert line not in owner_logs(service), line


def test_killing_one_members_pump_drops_only_that_cable_until_it_restarts(members, service, evidence):
    alpha, beta, network = members["alpha"], members["beta"], members["network"]
    beta_address = address_of(service, network, beta)
    serve_echo(service, beta)
    until_echo(service, alpha, beta_address, "the echo answers before the adversary")
    killed = guest(service, beta["id"], "pkill -9 -x capsem-tun; echo killed", check=False)
    assert "killed" in killed.get("stdout", ""), killed
    # The agent restarts the pump, the owner takes the fresh stream, and the
    # service plugs it again: bounded by the supervisor's backoff and the
    # plug handshake.
    wait_for(
        lambda: member_state(service, network, beta["id"]) != "ready" or not echo_answers(service, alpha, beta_address),
        "beta's cable is seen to drop",
        timeout=30,
    )
    recovery = until_echo(service, alpha, beta_address, "beta's cable is back", timeout=90)
    assert member_state(service, network, beta["id"]) == "ready"
    # Alpha's cable never dropped: the history shows exactly one close, beta's,
    # with the counters of the port the pump's death ended.
    closes = [event for event in network_events(service, network) if event["event_type"] == "network.close"]
    assert [event["event"]["network"]["source"]["vm"]["id"] for event in closes] == [beta["id"]], closes
    assert closes[0]["event"]["decision"]["reason"] == "closed"
    assert closes[0]["event"]["frames"]["frames_in"] > 0, closes[0]
    keep_recovery(evidence, "pump_kill", recovery)


def test_a_member_cannot_speak_as_another_address_and_its_broadcast_flood_is_capped(members, service):
    alpha, beta, network = members["alpha"], members["beta"], members["network"]
    alpha_address, beta_address = address_of(service, network, alpha), address_of(service, network, beta)
    serve_echo(service, beta)
    until_echo(service, alpha, beta_address, "the echo answers")

    # From its own address alpha is answered; from an address it gave itself
    # on the same cable, the switch drops every frame before beta sees one.
    assert answered_from(service, alpha, alpha_address, beta_address) >= 4
    forged = stranger_in(members["subnet"])
    cable = cable_of(service, alpha, alpha_address)
    guest(service, alpha["id"], f"ip addr add {forged}/32 dev {cable}")
    assert answered_from(service, alpha, forged, beta_address) == 0
    guest(service, alpha["id"], f"ip addr del {forged}/32 dev {cable}")

    # A broadcast flood: beta is still answered, and the switch stays small.
    (switch,) = switch_pids(service)
    before = resident_kib(switch)
    broadcast = str(ipaddress.IPv4Network(members["subnet"]).broadcast_address)
    start_in_guest(service, alpha["id"], "flood", f"python3 -c {shlex.quote(BROADCAST_FLOOD)} {broadcast} 6")
    time.sleep(1)
    during = udp(service, alpha, beta_address, 50, 64, wait_ms=1000)
    peak = resident_kib(switch)
    assert during["received"] > 0, during
    assert peak - before < FLOOD_GROWTH_KIB, (before, peak)
    wait_for(
        lambda: guest(service, alpha["id"], "cat /var/tmp/flood.log", check=False).get("stdout", "").strip().isdigit(),
        "the flood finished",
        timeout=30,
    )
    assert guest(service, beta["id"], "true")["exit_code"] == 0

    # Alpha's close row names both: the forged frames and the capped floods.
    leave(service, network, alpha)
    wait_for(lambda: cable_rows(service, network, "network.close", alpha["id"]), "alpha's close row", timeout=20)
    (closed,) = cable_rows(service, network, "network.close", alpha["id"])
    dropped = closed["event"]["frames"]["dropped"]
    assert dropped["source_address"] > 0 and dropped["storm"] > 0, dropped
    assert dropped["source_mac"] == 0, dropped


def test_plugging_and_unplugging_a_member_over_and_over_leaks_nothing(members, service):
    alpha, beta, network = members["alpha"], members["beta"], members["network"]
    serve_echo(service, beta)
    until_echo(service, alpha, address_of(service, network, beta), "the echo answers")
    (switch,) = switch_pids(service)
    sockets, _ = socket_descriptors(switch)
    before = resident_kib(switch)
    for _ in range(CHURN_ROUNDS):
        leave(service, network, beta)
        join(service, network, beta)
        linked(service, network, beta)
    until_echo(service, alpha, address_of(service, network, beta), "the churned member answers")
    # One switch, the same one, holding what it held before.
    assert switch_pids(service) == [switch]
    wait_for(lambda: socket_descriptors(switch)[0] == sockets, "the switch holds its cables and no more", timeout=20)
    assert resident_kib(switch) - before < FLOOD_GROWTH_KIB
    assert len(cables(service, beta)) == 1
    closes = cable_rows(service, network, "network.close", beta["id"])
    assert len(closes) == CHURN_ROUNDS, len(closes)


def test_deleting_a_network_with_members_is_refused_and_an_empty_one_takes_its_switch(members, service):
    alpha, beta, network = members["alpha"], members["beta"], members["network"]
    beta_address = address_of(service, network, beta)
    serve_echo(service, beta)
    until_echo(service, alpha, beta_address, "the echo answers")
    (switch,) = switch_pids(service)
    status, refusal = service.client().call_json("DELETE", f"/networks/{network}")
    assert status == 409, (status, refusal)
    assert len(network_info(service, network)["members"]) == 2
    assert echo_answers(service, alpha, beta_address)

    # Once empty, retiring the network stops its switch for good: the
    # process is reaped, not left waiting for grants (review finding 2).
    for vm in (alpha, beta):
        leave(service, network, vm)
    status, retired = service.client().call_json("DELETE", f"/networks/{network}")
    assert status == 200, (status, retired)
    wait_for(lambda: switch_pids(service) == [], "the retired network's switch is gone", timeout=20)
    with pytest.raises(ProcessLookupError):
        os.kill(switch, 0)
