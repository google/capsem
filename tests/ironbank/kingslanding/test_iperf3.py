"""Native iperf3 between two real members over their network's switch.

Two iperf3 containers in two VMs on one network, from a digest-pinned image
served by the hermetic registry. The server container listens; each client
container joins the network at creation and runs one short bounded transfer
by the server's private name: forward and reverse over TCP and one over UDP,
all frames on the same cables through the same switch. The JSON iperf3
prints is the evidence, kept with the run; this is a smoke of the rail the
benchmarks measure with, not a number to hold anything to.
"""

import contextlib
import json
import subprocess

import pytest
from helpers.constants import PROJECT_ROOT

from tests.fixtures.oci.registry import registry
from tests.ironbank.kingslanding.network import linked
from tests.ironbank.kingslanding.test_publish_benchmark import (
    IN_CONTAINER,
    evidence,
    guest,
    record,
)
from tests.ironbank.kingslanding.test_run import (
    command,
    create_command,
    environment,
    service,
    wait_for,
)

__all__ = ["evidence", "service"]
pytestmark = pytest.mark.integration

SERVER = "iperf-server"
NETWORK = "iperf"


@contextlib.contextmanager
def server(service, tmp_path, reference, certificate):
    """The iperf3 server container, a member of the network, up once listening."""
    result = subprocess.run(
        create_command(
            service, reference, certificate, "-n", SERVER, "--network", NETWORK
        ),
        env=environment(service),
        capture_output=True,
        timeout=240,
        check=False,
    )
    (tmp_path / "iperf-server.stderr").write_bytes(result.stderr)
    assert result.returncode == 0, result.stderr.decode(errors="replace")
    rows = [
        row
        for row in service.client().get("/vms/list")["sandboxes"]
        if row.get("name") == SERVER
    ]
    assert len(rows) == 1, rows
    try:
        # iperf3 buffers its "Server listening" line; the listening socket in
        # the container's namespace is the signal.
        def listening():
            sockets = guest(
                service,
                rows[0]["id"],
                f"{IN_CONTAINER} cat /proc/net/tcp",
                check=False,
            )
            return any(
                ":1451 " in line and " 0A " in line
                for line in sockets.get("stdout", "").splitlines()
            )

        wait_for(listening, "iperf3 server listening on 5201", timeout=180)
        yield rows[0]
    finally:
        for row in service.client().get("/vms/list")["sandboxes"]:
            with contextlib.suppress(Exception):
                service.client().delete(f"/vms/{row['id']}/delete")


def transfer(service, keep, reference, certificate, label, *iperf_args):
    """One iperf3 client container, a member from creation, run to its end;
    the JSON report it printed, kept under `keep`."""
    argv = command(service, reference, certificate, "--network", NETWORK)
    argv += ["-c", f"{SERVER}.{NETWORK}.capsem.internal", "-t", "2", "-J", *iperf_args]
    result = subprocess.run(
        argv, env=environment(service), capture_output=True, timeout=240, check=False
    )
    (keep / f"iperf-{label}.stdout").write_bytes(result.stdout)
    (keep / f"iperf-{label}.stderr").write_bytes(result.stderr)
    assert result.returncode == 0, result.stderr.decode(errors="replace")
    text = result.stdout.decode(errors="replace")
    report = json.loads(text[text.index("{") :])
    assert not report.get("error"), report
    return report


def test_native_iperf3_transfers_both_ways_over_the_switch(
    service, tmp_path, evidence
):
    api = service.client()
    network = api.post("/networks", {"name": NETWORK})["id"]
    with (
        registry(tmp_path, image="iperf3") as (reference, certificate, _),
        server(service, tmp_path, reference, certificate) as peer,
    ):
        linked(service, network, peer)
        forward = transfer(service, evidence, reference, certificate, "forward")
        assert forward["end"]["sum_received"]["bits_per_second"] > 1e6, forward["end"]
        reverse = transfer(service, evidence, reference, certificate, "reverse", "-R")
        assert reverse["end"]["sum_received"]["bits_per_second"] > 1e6, reverse["end"]
        udp = transfer(
            service, evidence, reference, certificate, "udp", "-u", "-b", "50M"
        )
        assert udp["end"]["sum"]["packets"] > 0, udp["end"]
        assert udp["end"]["sum"]["lost_percent"] < 50, udp["end"]
        # The history names cables, never flows: nothing admitted TCP
        # per connection, so no row but the cables' own.
        protocols = {
            event["event"]["network"].get("protocol")
            for event in api.get(f"/networks/{network}/logs").get("events", [])
        }
        assert protocols == {"link"}, protocols
        # The native numbers join the benchmark store beside the bench's own.
        metrics = {
            "iperf3.forward.megabits_per_sec": {
                "unit": "megabits_per_second",
                "samples": [forward["end"]["sum_received"]["bits_per_second"] / 1e6],
            },
            "iperf3.forward.retransmits": {
                "unit": "count",
                "samples": [forward["end"]["sum_sent"].get("retransmits", 0)],
            },
            "iperf3.reverse.megabits_per_sec": {
                "unit": "megabits_per_second",
                "samples": [reverse["end"]["sum_received"]["bits_per_second"] / 1e6],
            },
            "iperf3.reverse.retransmits": {
                "unit": "count",
                "samples": [reverse["end"]["sum_sent"].get("retransmits", 0)],
            },
            "iperf3.udp.megabits_per_sec": {
                "unit": "megabits_per_second",
                "samples": [udp["end"]["sum"]["bits_per_second"] / 1e6],
            },
            "iperf3.udp.lost_ratio": {
                "unit": "ratio",
                "samples": [udp["end"]["sum"]["lost_percent"] / 100.0],
            },
            "iperf3.udp.jitter_ms": {
                "unit": "milliseconds",
                "samples": [udp["end"]["sum"]["jitter_ms"]],
            },
        }
        source_commit = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=PROJECT_ROOT, timeout=5, text=True
        ).strip()
        record(evidence, metrics, source_commit)
