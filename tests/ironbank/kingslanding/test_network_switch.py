"""Members of a network talk over its switch: every protocol, one path.

Two container VMs join one network. Each gets an address in the network's
subnet and one cable into the network's switch; TCP, UDP and ICMP between the
containers, and the ARP that finds each peer, all cross that one switch --
there is no other private path. TCP keeps its semantics end to end (half
close, reset, a large last write), UDP fragments cross, a stranger's address
gets nothing, and leaving cuts even a flow that is mid-transfer. A VM on two
networks talks on both, each switch holds only its own network's cables, and
the VM never carries one network's packets onto the other: they reach it
and are dropped. The network's history names every cable, with its frame
counters when it closes.
"""

import ipaddress
import shlex

import pytest

from tests.ironbank.kingslanding.network import (
    CABLE_MTU,
    CABLE_SPEED_MBPS,
    ECHO_PORT,
    address_of,
    another,
    bench_in,
    cable_of,
    cable_rows,
    cables,
    echo_answers,
    join,
    leave,
    linked,
    mac_of,
    member_state,
    members,
    network_info,
    ping,
    serve_echo,
    socket_descriptors,
    stranger_in,
    switch_pids,
    udp,
)
from tests.ironbank.kingslanding.test_publish_benchmark import (
    IN_CONTAINER,
    THROUGHPUT_PORT,
    client_args,
    evidence,
    guest,
    start_in_guest,
)
from tests.ironbank.kingslanding.test_run import service, wait_for

__all__ = ["evidence", "members", "service"]
pytestmark = pytest.mark.integration

STREAM_PORT = 5301
# One TCP server per mode, in the peer's container:
#   late   reads the whole request after the client's half close, thinks, answers
#   upload says everything and half closes first, then reads the whole upload
#   reset  reads until the client aborts, and says how the stream ended
#   final  writes a large last payload and closes at once
#   digest reads a large upload to its end and answers with its digest
STREAM_SERVER = """
import hashlib, socket, sys, time
mode, port = sys.argv[1], int(sys.argv[2])
listener = socket.socket()
listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
listener.bind(("0.0.0.0", port))
listener.listen()
open(f"/var/tmp/stream-{mode}.ready", "w").close()
payload = bytes(range(256)) * (16 * 1024 * 1024 // 256)
while True:
    peer, _ = listener.accept()
    if mode == "late":
        size = 0
        while chunk := peer.recv(65536):
            size += len(chunk)
        time.sleep(3)
        peer.sendall(f"late reply to {size}".encode())
    elif mode == "upload":
        peer.sendall(b"hello")
        peer.shutdown(socket.SHUT_WR)
        size = 0
        while chunk := peer.recv(65536):
            size += len(chunk)
        open("/var/tmp/stream-upload.size", "w").write(str(size))
    elif mode == "reset":
        try:
            while peer.recv(65536):
                pass
            outcome = "fin"
        except ConnectionResetError:
            outcome = "reset"
        open("/var/tmp/stream-reset.outcome", "w").write(outcome)
    elif mode == "final":
        peer.sendall(payload)
    elif mode == "digest":
        digest = hashlib.sha256()
        while chunk := peer.recv(1 << 20):
            digest.update(chunk)
        peer.sendall(digest.hexdigest().encode())
    peer.close()
"""
STREAM_CLIENT = """
import hashlib, socket, struct, sys, time
mode, address, port = sys.argv[1], sys.argv[2], int(sys.argv[3])
stream = socket.create_connection((address, port), timeout=30)
payload = bytes(range(256)) * (16 * 1024 * 1024 // 256)
if mode == "late":
    stream.sendall(b"q" * 1000)
    stream.shutdown(socket.SHUT_WR)
elif mode == "upload":
    assert stream.recv(5) == b"hello"
    assert stream.recv(1) == b""
    time.sleep(2)
    stream.sendall(b"u" * 200_000)
    stream.shutdown(socket.SHUT_WR)
elif mode == "reset":
    stream.sendall(b"r" * 1000)
    time.sleep(1)
    stream.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack("ii", 1, 0))
    stream.close()
    print("aborted")
    sys.exit(0)
elif mode == "final":
    digest, size = hashlib.sha256(), 0
    while chunk := stream.recv(1 << 20):
        digest.update(chunk)
        size += len(chunk)
    print(size, digest.hexdigest() == hashlib.sha256(payload).hexdigest())
    sys.exit(0)
elif mode == "digest":
    stream.sendall(payload)
    stream.shutdown(socket.SHUT_WR)
    print(stream.recv(64).decode() == hashlib.sha256(payload).hexdigest())
    sys.exit(0)
reply = b""
while chunk := stream.recv(65536):
    reply += chunk
print(reply.decode())
"""
# Sends for as long as the peer takes it, then says it stalled.
STALLING_CLIENT = """
import socket, sys
stream = socket.create_connection((sys.argv[1], int(sys.argv[2])), timeout=30)
stream.settimeout(3)
sent = 0
try:
    while True:
        sent += stream.send(b"s" * 65536)
        if sent >= 1 << 20:
            open("/var/tmp/stalling.flowing", "w").close()
except OSError as error:
    print(f"stalled after {sent} bytes: {error}")
"""
MODES = ("late", "upload", "reset", "final", "digest")


def serve_streams(service, vm):
    for offset, mode in enumerate(MODES):
        start_in_guest(
            service,
            vm["id"],
            f"stream-{mode}",
            f"{IN_CONTAINER} python3 -c {shlex.quote(STREAM_SERVER)} {mode} {STREAM_PORT + offset}",
        )
    for mode in MODES:
        wait_for(
            lambda mode=mode: exists(service, vm, f"/var/tmp/stream-{mode}.ready"),
            f"the {mode} stream server listens",
            timeout=20,
        )


def exists(service, vm, path):
    return guest(service, vm["id"], f"test -e {path}", check=False).get("exit_code") == 0


def read(service, vm, path):
    return guest(service, vm["id"], f"cat {path}", check=False).get("stdout", "").strip()


def stream_client(service, vm, mode, address):
    port = STREAM_PORT + MODES.index(mode)
    result = guest(
        service,
        vm["id"],
        f"{IN_CONTAINER} python3 -c {shlex.quote(STREAM_CLIENT)} {mode} {address} {port}",
        timeout=60,
        check=False,
    )
    assert result.get("exit_code") == 0, (mode, result)
    return result["stdout"].strip()


def transit_drops(service, vm):
    """Packets `vm` dropped rather than forward from one cable to another."""
    chain = guest(service, vm["id"], "iptables-nft -L FORWARD -v -x -n")["stdout"]
    rules = [line.split() for line in chain.splitlines()]
    (packets,) = [int(rule[0]) for rule in rules if "DROP" in rule and rule.count("cable+") == 2]
    return packets


def rx_packets(service, vm, device):
    return int(read(service, vm, f"/sys/class/net/{device}/statistics/rx_packets"))


def throughput(service, vm, address, direction, streams, seconds=2):
    return bench_in(
        service,
        vm,
        *client_args(direction, streams, seconds=seconds),
        "--address",
        f"{address}:{THROUGHPUT_PORT}",
        timeout=40,
    )


def test_members_speak_tcp_udp_icmp_and_arp_over_their_networks_switch(members, service):
    alpha, beta, network = members["alpha"], members["beta"], members["network"]
    subnet = ipaddress.IPv4Network(members["subnet"])
    alpha_address, beta_address = address_of(service, network, alpha), address_of(service, network, beta)
    assert alpha_address != beta_address
    assert {ipaddress.IPv4Address(alpha_address), ipaddress.IPv4Address(beta_address)} <= set(subnet.hosts())
    assert [member["state"] for member in network_info(service, network)["members"]] == ["ready", "ready"]

    # One switch for the network; each VM has exactly the one cable, holding
    # its lease with the subnet's prefix, declared at 10 Gb/s.
    assert len(switch_pids(service)) == 1
    for vm, address in ((alpha, alpha_address), (beta, beta_address)):
        assert cables(service, vm) == {cable_of(service, vm, address): f"{address}/{subnet.prefixlen}"}
        device = cable_of(service, vm, address)
        sysfs = guest(
            service,
            vm["id"],
            f"cat /sys/class/net/{device}/speed /sys/class/net/{device}/duplex /sys/class/net/{device}/mtu",
        )["stdout"].split()
        assert sysfs == [CABLE_SPEED_MBPS, "full", CABLE_MTU], (vm["id"], sysfs)

    # TCP: bulk both ways, then the stream semantics a relay would bend.
    start_in_guest(
        service,
        beta["id"],
        "throughput-server",
        f"{IN_CONTAINER} capsem-bench-rs throughput --serve 0.0.0.0:{THROUGHPUT_PORT}",
    )
    wait_for(
        lambda: bench_in(
            service, alpha, *client_args("latency", 1, seconds=1), "--address",
            f"{beta_address}:{THROUGHPUT_PORT}", timeout=15, check=False,
        ).get("exit_code") == 0,
        "alpha reaches beta's throughput server over the switch",
        timeout=60,
    )
    for direction in ("upload", "download", "bidirectional"):
        throughput(service, alpha, beta_address, direction, 4)
    serve_streams(service, beta)
    assert stream_client(service, alpha, "late", beta_address) == "late reply to 1000"
    stream_client(service, alpha, "upload", beta_address)
    wait_for(
        lambda: read(service, beta, "/var/tmp/stream-upload.size") == "200000",
        "beta read alpha's whole upload after it stopped speaking",
        timeout=20,
    )
    assert stream_client(service, alpha, "reset", beta_address) == "aborted"
    wait_for(
        lambda: read(service, beta, "/var/tmp/stream-reset.outcome") == "reset",
        "beta saw the reset, not a clean end",
        timeout=20,
    )
    assert stream_client(service, alpha, "final", beta_address) == f"{16 * 1024 * 1024} True"
    assert stream_client(service, alpha, "digest", beta_address) == "True"

    # UDP: ordinary, frame-sized, and a datagram the sender fragments.
    serve_echo(service, beta)
    wait_for(lambda: echo_answers(service, alpha, beta_address), "beta's echo answers", timeout=60)
    small = udp(service, alpha, beta_address, 200, 1400)
    assert small["received"] >= 195 and max(small["round_trips"]) < 500, small
    assert udp(service, alpha, beta_address, 20, 60_000)["received"] >= 19
    assert udp(service, alpha, beta_address, 10, 65_507)["received"] >= 9

    # ICMP echo, answered in beta's container.
    assert ping(service, alpha, beta_address, 20)["received"] >= 19

    # ARP found the peer through the switch's flood: each guest knows the
    # other's MAC, which is the peer's address.
    for vm, peer in ((beta, alpha_address), (alpha, beta_address)):
        neighbour = guest(service, vm["id"], f"ip neigh show {peer}")["stdout"]
        assert f"lladdr {mac_of(peer)}" in neighbour, (vm["id"], neighbour)

    # Names on the private zone resolve for members of the asker's networks.
    assert udp(service, alpha, "beta.team.capsem.internal", 20, 64, by_name=True)["received"] >= 19
    unresolved = bench_in(
        service, alpha, "udp", "--address", f"stranger.team.capsem.internal:{ECHO_PORT}",
        "--count", "1", by_name=True, check=False,
    )
    assert unresolved.get("exit_code") not in (None, 0), unresolved

    # A stranger's address inside the subnet: no ARP answer, so nothing.
    stranger = stranger_in(members["subnet"])
    assert udp(service, alpha, stranger, 5, 64, wait_ms=1000)["received"] == 0
    assert ping(service, alpha, stranger, 3, timeout_ms=500)["received"] == 0

    # The history names both cables as linked on this network.
    for vm, address in ((alpha, alpha_address), (beta, beta_address)):
        linked_rows = cable_rows(service, network, "network.connect", vm["id"])
        assert linked_rows, vm["id"]
        facts = linked_rows[-1]["event"]
        assert facts["network"]["context"] == "private"
        assert facts["network"]["source"] == {"vm": {"id": vm["id"]}, "address": address, "port": 0}
        assert facts["decision"] == {"effective": "allow", "reason": "linked"}


def test_leaving_cuts_even_a_flowing_stream_and_joining_again_plugs_a_fresh_cable(members, service):
    alpha, beta, network = members["alpha"], members["beta"], members["network"]
    beta_address = address_of(service, network, beta)
    serve_echo(service, beta)
    start_in_guest(
        service,
        beta["id"],
        "sink",
        f"{IN_CONTAINER} python3 -c {shlex.quote(STREAM_SERVER)} digest {STREAM_PORT}",
    )
    wait_for(lambda: echo_answers(service, alpha, beta_address), "beta's echo answers", timeout=60)
    wait_for(lambda: exists(service, beta, "/var/tmp/stream-digest.ready"), "beta's sink listens", timeout=20)

    # A stream is mid-transfer when beta leaves: the leave ends it, rather than
    # letting an existing flow outlive the membership.
    start_in_guest(
        service,
        alpha["id"],
        "stalling",
        f"{IN_CONTAINER} python3 -c {shlex.quote(STALLING_CLIENT)} {beta_address} {STREAM_PORT}",
    )
    wait_for(lambda: exists(service, alpha, "/var/tmp/stalling.flowing"), "the stream flows", timeout=30)
    leave(service, network, beta)
    assert member_state(service, network, beta["id"]) == "absent"
    wait_for(
        lambda: "stalled after" in read(service, alpha, "/var/tmp/stalling.log"),
        "the stream stalls once beta has left",
        timeout=30,
    )
    assert udp(service, alpha, beta_address, 5, 64, wait_ms=1000)["received"] == 0
    wait_for(lambda: cables(service, beta) == {}, "beta's guest took the cable down", timeout=20)

    # The close row carries the switch's counters: frames crossed both ways.
    wait_for(
        lambda: cable_rows(service, network, "network.close", beta["id"]),
        "the leave's close row",
        timeout=20,
    )
    (closed,) = cable_rows(service, network, "network.close", beta["id"])
    assert closed["event"]["decision"] == {"effective": "allow", "reason": "unlinked"}
    frames = closed["event"]["frames"]
    assert frames["reason"] == "Cancelled", frames
    assert frames["frames_in"] > 0 and frames["frames_out"] > 0, frames
    assert frames["bytes_in"] > frames["frames_in"] and frames["bytes_out"] > frames["frames_out"], frames
    assert set(frames["dropped"]) == {"short", "source_mac", "source_address", "unknown", "queue_full", "storm"}
    assert frames["dropped"]["source_mac"] == 0 and frames["dropped"]["source_address"] == 0, frames

    # Joining again plugs a fresh cable, at whatever address the lease names.
    join(service, network, beta)
    linked(service, network, beta)
    again = address_of(service, network, beta)
    assert list(cables(service, beta).values()) == [f"{again}/{ipaddress.IPv4Network(members['subnet']).prefixlen}"]
    wait_for(lambda: echo_answers(service, alpha, again), "the rejoined member answers again", timeout=60)

    # A burst past the switch's queues loses frames, never a VM.
    burst = udp(service, alpha, again, 5000, 1400, interval_ms=0, wait_ms=3000)
    assert burst["received"] > 0, burst
    assert guest(service, beta["id"], "true")["exit_code"] == 0
    assert guest(service, alpha["id"], "true")["exit_code"] == 0


def test_a_vm_on_two_networks_talks_on_both_and_carries_nothing_between_them(members, service, evidence):
    alpha, beta, team = members["alpha"], members["beta"], members["network"]
    (team_switch,) = switch_pids(service)
    team_sockets, listing = socket_descriptors(team_switch)
    (evidence / "team-switch-descriptors.txt").write_text(listing)
    with another(service, members, "gamma") as gamma:
        other = service.client().post("/networks", {"name": "other"})
        assert other["subnet"] != members["subnet"]
        for vm in (alpha, gamma):
            join(service, other["id"], vm)
        linked(service, other["id"], alpha, gamma)

        # Two networks, two switches: the new one is the other network's.
        (other_switch,) = set(switch_pids(service)) - {team_switch}
        other_sockets, listing = socket_descriptors(other_switch)
        (evidence / "other-switch-descriptors.txt").write_text(listing)
        assert socket_descriptors(team_switch)[0] == team_sockets

        # Alpha has one cable per network, each holding that network's lease.
        on_team, on_other = address_of(service, team, alpha), address_of(service, other["id"], alpha)
        assert ipaddress.IPv4Address(on_other) in ipaddress.IPv4Network(other["subnet"])
        assert sorted(cables(service, alpha).values()) == sorted(
            [f"{on_team}/{ipaddress.IPv4Network(members['subnet']).prefixlen}",
             f"{on_other}/{ipaddress.IPv4Network(other['subnet']).prefixlen}"]
        )
        beta_address, gamma_address = address_of(service, team, beta), address_of(service, other["id"], gamma)
        serve_echo(service, beta)
        serve_echo(service, gamma)
        wait_for(lambda: echo_answers(service, alpha, beta_address), "alpha reaches beta on team", timeout=60)
        wait_for(lambda: echo_answers(service, alpha, gamma_address), "alpha reaches gamma on other", timeout=60)

        # Beta routes the other network through alpha. The packets reach
        # alpha -- its cable-to-cable drop counts them -- and gamma's cable
        # receives none of them.
        beta_cable = cable_of(service, beta, beta_address)
        gamma_cable = cable_of(service, gamma, gamma_address)
        guest(service, beta["id"], f"ip route add {other['subnet']} via {on_team} dev {beta_cable}")
        dropped, received = transit_drops(service, alpha), rx_packets(service, gamma, gamma_cable)
        assert udp(service, beta, gamma_address, 50, 64, wait_ms=1000)["received"] == 0
        assert ping(service, beta, gamma_address, 3, timeout_ms=500)["received"] == 0
        assert transit_drops(service, alpha) - dropped >= 50
        assert rx_packets(service, gamma, gamma_cable) - received < 5

        # Gamma leaving its network closes one cable on its own switch only.
        leave(service, other["id"], gamma)
        wait_for(
            lambda: socket_descriptors(other_switch)[0] == other_sockets - 1,
            "the other network's switch closed gamma's cable",
            timeout=20,
        )
        assert socket_descriptors(team_switch)[0] == team_sockets
        assert echo_answers(service, alpha, beta_address)
        leave(service, other["id"], alpha)
        service.client().delete(f"/networks/{other['id']}")
        wait_for(lambda: switch_pids(service) == [team_switch], "the retired network's switch is gone", timeout=20)
