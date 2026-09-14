"""A burst of host-side resets on a published port must not take the VM down.

Three of five private-link recordings failed at the end of a bidirectional
trial: the host client reset sixteen connections at once, the guest reported
each close, and the owner's reset of a source the kernel had already torn
down failed with EINVAL. That error ended the control link, and with it
every VSOCK link of the VM in the same millisecond (S04-018). This drives the
same burst three times and asserts the links survive each one.
"""

import pytest

from tests.ironbank.kingslanding.test_private_link_benchmark import (
    BENCH,
    IN_CONTAINER,
    THROUGHPUT_PORT,
    client_args,
    container,
    evidence,
    guest,
    probe,
    service,
    start_in_guest,
)
from tests.ironbank.kingslanding.test_run import wait_for

__all__ = ["container", "evidence", "service"]
pytestmark = pytest.mark.integration

ROUNDS = 3
STREAMS = 16
# What the owner logged when the link collapsed; none of it may appear.
COLLAPSE_LINES = (
    "virtual machine stopped",
    "guest close report rejected",
    "guest control lease missing",
)


def owner_log(service):
    return "".join(
        log.read_text(errors="replace")
        for log in service.tmp_dir.glob("persistent/*/process.log")
    )


def test_a_reset_burst_on_a_published_port_leaves_every_guest_link_up(
    container, service
):
    vm_id = container["vm"]["id"]
    published = f"127.0.0.1:{container['port']}"
    start_in_guest(
        service,
        vm_id,
        "throughput-server",
        f"{IN_CONTAINER} capsem-bench-rs throughput --serve 0.0.0.0:{THROUGHPUT_PORT}",
    )
    latency = [BENCH, *client_args("latency", 1, seconds=1), "--address", published]
    wait_for(
        lambda: probe(latency).returncode == 0,
        "published throughput server",
        timeout=30,
    )

    for round_number in range(1, ROUNDS + 1):
        burst = probe(
            [
                BENCH,
                *client_args("bidirectional", STREAMS, seconds=3),
                "--address",
                published,
            ]
        )
        # The client itself saw the collapse first: a broken pipe mid-trial.
        assert burst.returncode == 0, f"round {round_number}: {burst.stderr}"
        # The control lease: a fresh publication is set up and answers.
        again = probe(latency)
        assert again.returncode == 0, f"round {round_number}: {again.stderr}"
        # Exec and the link: the guest answers and still holds tap0.
        device = guest(service, vm_id, "ip -o addr show tap0", timeout=20)
        assert f"inet {container['vm']['private_address']}/9" in device["stdout"], (
            device
        )
        log = owner_log(service)
        for line in COLLAPSE_LINES:
            assert line not in log, f"round {round_number}: owner logged {line!r}"
