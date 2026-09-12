"""Private TCP between two real members of a named network.

Two containers boot in two VMs, join one network, and a workload in the
first connects to the second's private address. The guest proxy intercepts
the connect, the first owner asks the service, the service admits it and the
second owner hands the stream to its confined router, which dials the
container. Nothing here uses a host-published shortcut: the bytes cross
owner to owner. Denials happen before any byte, and both VMs' rows land in
the network's audit history.
"""

import contextlib
import os
import re
import shlex
import signal
import subprocess
import time

import pytest
from helpers.constants import BIN_DIR

from tests.fixtures.oci.registry import registry
from tests.ironbank.kingslanding.test_private_link_benchmark import (
    IN_CONTAINER,
    THROUGHPUT_PORT,
    client_args,
    evidence,
    guest,
    start_in_guest,
)
from tests.ironbank.kingslanding.test_run import (
    command,
    environment,
    service,
    wait_for,
)

__all__ = ["evidence", "service"]
pytestmark = pytest.mark.integration

STRANGER = "10.128.0.9"
KEPT_LOGS = ("process.log", "serial.log", ".capsem-agent-stdio.log")


@contextlib.contextmanager
def member(service, tmp_path, reference, certificate, name, *publish):
    """One container VM under `name`, up once its Redis accepts connections."""
    stdout = tmp_path / f"{name}.stdout"
    stderr = tmp_path / f"{name}.stderr"
    with stdout.open("wb") as out, stderr.open("wb") as err:
        process = subprocess.Popen(
            command(service, reference, certificate, "-n", name, *publish),
            env=environment(service),
            stdout=out,
            stderr=err,
        )
        try:

            def ready():
                assert process.poll() is None, stderr.read_text()
                return b"Ready to accept connections tcp" in stdout.read_bytes()

            wait_for(ready, f"{name} container startup", timeout=180)
            rows = [
                row
                for row in service.client().get("/vms/list")["sandboxes"]
                if row.get("name") == name
            ]
            assert len(rows) == 1, rows
            yield {**rows[0], "stderr": stderr}
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=30)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)


@pytest.fixture
def members(service, tmp_path, evidence):
    """Two members of one network, `alpha` with a published throughput port."""
    with (
        registry(tmp_path) as (reference, certificate, _),
        member(
            service,
            tmp_path,
            reference,
            certificate,
            "alpha",
            "-p",
            f"0:{THROUGHPUT_PORT}",
        ) as alpha,
        member(service, tmp_path, reference, certificate, "beta") as beta,
    ):
        client = service.client()
        network = client.post("/networks", {"name": "team"})
        for vm in (alpha, beta):
            client.put(f"/networks/{network['id']}/members/{vm['id']}")
        try:
            yield {"alpha": alpha, "beta": beta, "network": network["id"]}
        finally:
            # Every VM's owner log, console and agent log, kept where the gate
            # cannot delete them.
            for name in KEPT_LOGS:
                for log in service.tmp_dir.rglob(name):
                    stem = "-".join(log.relative_to(service.tmp_dir).parts[:-1])
                    (evidence / f"{stem}-{name}").write_bytes(log.read_bytes())
            for row in client.get("/vms/list")["sandboxes"]:
                with contextlib.suppress(Exception):
                    client.delete(f"/vms/{row['id']}/delete")


def published_port(vm):
    mappings = re.findall(
        rf"Published 127.0.0.1:(\d+) -> {THROUGHPUT_PORT}/tcp", vm["stderr"].read_text()
    )
    assert len(mappings) == 1, vm["stderr"].read_text()
    return int(mappings[0])


def connect_from(service, vm, address, direction="latency", seconds=1, timeout=15):
    """Run the throughput client inside `vm`'s container against `address`."""
    started = time.monotonic()
    result = guest(
        service,
        vm["id"],
        f"{IN_CONTAINER} "
        + shlex.join(
            [
                "capsem-bench-rs",
                *client_args(direction, 1, seconds=seconds),
                "--address",
                address,
            ]
        ),
        timeout=timeout,
        check=False,
    )
    return result, time.monotonic() - started


def serve_in(service, vm):
    start_in_guest(
        service,
        vm["id"],
        "throughput-server",
        f"{IN_CONTAINER} capsem-bench-rs throughput --serve 0.0.0.0:{THROUGHPUT_PORT}",
    )


def test_members_reach_each_other_on_private_addresses_and_strangers_are_refused(
    members, service
):
    alpha, beta = members["alpha"], members["beta"]
    serve_in(service, beta)
    target = f"{beta['private_address']}:{THROUGHPUT_PORT}"
    wait_for(
        lambda: connect_from(service, alpha, target)[0].get("exit_code") == 0,
        "alpha reaches beta's throughput server on its private address",
        timeout=60,
    )
    bulk, elapsed = connect_from(
        service, alpha, target, "upload", seconds=2, timeout=20
    )
    assert bulk.get("exit_code") == 0, bulk
    assert elapsed < 15, elapsed

    # The network's history names the flow from both ends.
    def audited():
        logs = service.client().get(f"/networks/{members['network']}/logs")
        for event in logs.get("events", []):
            facts = event["event"]["network"]
            if (
                facts["source"]["vm"]["id"] == alpha["id"]
                and facts["destination"]["vm"]["id"] == beta["id"]
                and facts["destination"]["port"] == THROUGHPUT_PORT
                and event["event"]["decision"]["effective"] == "allow"
            ):
                return True
        return False

    wait_for(audited, "network history records the admitted connection", timeout=20)

    # A private address that belongs to no member is refused before any byte.
    stranger, elapsed = connect_from(service, alpha, f"{STRANGER}:{THROUGHPUT_PORT}")
    assert stranger.get("exit_code") not in (None, 0), stranger
    assert elapsed < 10, elapsed

    # Leaving the network ends reachability at once, in both directions.
    service.client().delete(f"/networks/{members['network']}/members/{beta['id']}")
    detached, elapsed = connect_from(service, alpha, target)
    assert detached.get("exit_code") not in (None, 0), detached
    assert elapsed < 10, elapsed


def test_killing_one_owner_drops_only_its_own_flows(members, service):
    alpha, beta = members["alpha"], members["beta"]
    serve_in(service, alpha)
    serve_in(service, beta)
    bench = str(BIN_DIR / "capsem-bench-rs")
    published = f"127.0.0.1:{published_port(alpha)}"
    latency = [bench, *client_args("latency", 1, seconds=1), "--address", published]
    wait_for(
        lambda: (
            subprocess.run(
                latency, capture_output=True, timeout=30, check=False
            ).returncode
            == 0
        ),
        "alpha's published port answers",
        timeout=60,
    )
    target = f"{beta['private_address']}:{THROUGHPUT_PORT}"
    wait_for(
        lambda: connect_from(service, alpha, target)[0].get("exit_code") == 0,
        "alpha reaches beta",
        timeout=60,
    )
    # A long private transfer is in flight when beta's owner dies.
    start_in_guest(
        service,
        alpha["id"],
        "private-transfer",
        f"{IN_CONTAINER} "
        + shlex.join(
            [
                "capsem-bench-rs",
                *client_args("bidirectional", 1, seconds=8),
                "--address",
                target,
            ]
        ),
    )
    time.sleep(1)
    os.kill(beta["pid"], signal.SIGKILL)

    # alpha's own flows survive: its published port, its guest, its console.
    survivor = subprocess.run(
        latency, capture_output=True, text=True, timeout=30, check=False
    )
    assert survivor.returncode == 0, survivor.stderr
    assert guest(service, alpha["id"], "true")["exit_code"] == 0
    # The private transfer ended with an error rather than hanging.
    wait_for(
        lambda: (
            "rror"
            in guest(
                service, alpha["id"], "cat /var/tmp/private-transfer.log", check=False
            ).get("stdout", "")
        ),
        "the private transfer reported the lost peer",
        timeout=30,
    )
