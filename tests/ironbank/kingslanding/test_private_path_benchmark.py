"""Exploratory private-path measurements between two real members.

The same `capsem-bench-rs` clients the published lane uses, run from the
first member's container against the second member's private address:
bulk TCP over the per-connection admitted path (every direction and stream
count of the published matrix), UDP round trips and ICMP echo over the
frame link, and Redis requests over private TCP. Everything is recorded
through the benchmark store beside the published-port and Redis lanes so
the paths compare on one report. Exploratory: no release baseline, no
threshold, and the doctor's verdict on the machine travels with the run.
"""

import hashlib
import json
import shlex
import subprocess

import pytest
from helpers.constants import ASSETS_DIR, BIN_DIR, PROJECT_ROOT

from tests.ironbank.kingslanding.test_private_datagram import (
    ECHO_PORT,
    linked,
    probe,
    udp,
)
from tests.ironbank.kingslanding.test_private_link_benchmark import (
    BENCH,
    IN_CONTAINER,
    MATRIX,
    THROUGHPUT_PORT,
    TRIALS,
    client_args,
    evidence,
    guest,
    record,
    start_in_guest,
)
from tests.ironbank.kingslanding.test_private_tcp import members
from tests.ironbank.kingslanding.test_run import service, wait_for

__all__ = ["evidence", "members", "service"]
pytestmark = pytest.mark.integration

REDIS_PORT = 6379


def in_container(service, vm, argv, timeout):
    """One bench client inside `vm`'s container; its JSON report as text."""
    result = guest(
        service,
        vm["id"],
        f"{IN_CONTAINER} " + shlex.join(["capsem-bench-rs", *argv]),
        timeout=timeout,
        check=False,
    )
    assert result.get("exit_code") == 0, result
    return result["stdout"]


def keep(output, identity, metrics, label, args, raw, settings_key):
    document = json.loads(raw)
    (output / f"{label}.json").write_text(raw)
    identity["trials"].append(
        {"label": label, "args": args, "settings": document[settings_key]}
    )
    lane = label.rsplit(".", 1)[0]
    for name, metric in document["metrics"].items():
        metrics.setdefault(f"{lane}.{name}", {"unit": metric["unit"], "samples": []})[
            "samples"
        ].extend(metric["samples"])
    return document


def test_private_path_transport_samples(members, service, evidence):
    alpha, beta = members["alpha"], members["beta"]
    output = evidence
    doctor = subprocess.run(
        [BENCH, "doctor", "--json"], capture_output=True, timeout=15, check=False
    )
    (output / "doctor.json").write_bytes(doctor.stdout)
    identity = {
        "doctor_exit": doctor.returncode,
        "source_commit": subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=PROJECT_ROOT, timeout=5, text=True
        ).strip(),
        "source_diff_sha256": hashlib.sha256(
            subprocess.check_output(
                ["git", "diff", "HEAD"], cwd=PROJECT_ROOT, timeout=5
            )
        ).hexdigest(),
        "manifest": json.loads((ASSETS_DIR / "manifest.json").read_text()),
        "host_binaries": {
            name: hashlib.sha256((BIN_DIR / name).read_bytes()).hexdigest()
            for name in (
                "capsem-bench-rs",
                "capsem-process",
                "capsem-router",
                "capsem-service",
            )
        },
        "lanes": {
            "private_tcp": "member container -> private address -> admitted per connection -> destination owner's router -> VSOCK -> member container",
            "private_udp": "member container -> tap0 -> VSOCK -> network switch -> VSOCK -> tap0 -> member container, echoed back",
            "private_ping": "member container -> the link -> the peer VM's kernel, echoed back",
            "private_redis": "member container Redis client -> private address -> admitted per connection -> Redis in the peer's container",
        },
        "classification": "exploratory; no release baseline or performance threshold",
        "trials": [],
    }
    network = members["network"]
    linked(service, network, alpha, beta)
    start_in_guest(
        service,
        beta["id"],
        "throughput-server",
        f"{IN_CONTAINER} capsem-bench-rs throughput --serve 0.0.0.0:{THROUGHPUT_PORT}",
    )
    start_in_guest(
        service,
        beta["id"],
        "udp-echo",
        f"{IN_CONTAINER} capsem-bench-rs udp --serve 0.0.0.0:{ECHO_PORT}",
    )
    target = f"{beta['private_address']}:{THROUGHPUT_PORT}"
    wait_for(
        lambda: (
            guest(
                service,
                alpha["id"],
                f"{IN_CONTAINER} "
                + shlex.join(
                    [
                        "capsem-bench-rs",
                        *client_args("latency", 1, seconds=1),
                        "--address",
                        target,
                    ]
                ),
                timeout=15,
                check=False,
            ).get("exit_code")
            == 0
        ),
        "the peer's throughput server answers on its private address",
        timeout=60,
    )

    metrics = {}
    try:
        for direction, streams in MATRIX:
            for trial in range(TRIALS):
                args = client_args(direction, streams)
                raw = in_container(
                    service, alpha, [*args, "--address", target], timeout=40
                )
                keep(
                    output,
                    identity,
                    metrics,
                    f"private_tcp.{direction}.s{streams}.r{trial}",
                    args,
                    raw,
                    "throughput",
                )
        for _trial in range(TRIALS):
            udp(
                service,
                alpha,
                beta["private_address"],
                200,
                1400,
                recorded=metrics,
                lane="private_udp.b1400",
            )
            probe(
                service,
                alpha,
                "ping",
                "--address",
                beta["private_address"],
                "--count",
                "20",
                "--interval-ms",
                "20",
                recorded=metrics,
                lane="private_ping",
            )
        for trial in range(TRIALS):
            args = [
                "redis",
                "--requests",
                "10000",
                "--concurrency",
                "32",
                "--pipeline",
                "16",
            ]
            raw = in_container(
                service,
                alpha,
                [*args, "--address", f"{beta['private_address']}:{REDIS_PORT}"],
                timeout=60,
            )
            document = keep(
                output,
                identity,
                metrics,
                f"private_redis.c32.p16.r{trial}",
                args,
                raw,
                "redis",
            )
            assert document["metrics"]["completed_requests"]["samples"] == [10000]
    finally:
        for name in ("throughput-server", "udp-echo"):
            log = guest(service, beta["id"], f"cat /var/tmp/{name}.log", check=False)
            (output / f"{name}.log").write_text(
                log.get("stdout", "") + log.get("stderr", "")
                if log.get("exit_code") == 0
                else json.dumps(log, indent=2)
            )
    (output / "identity.json").write_text(json.dumps(identity, indent=2) + "\n")
    record(output, metrics, identity["source_commit"])
