"""Exploratory measurements between two members over their network's switch.

The same `capsem-bench-rs` clients the published lane uses, run from the
first member's container against the second member's address: bulk TCP in
every direction and stream count of the published matrix, UDP round trips,
ICMP echo and Redis requests -- all of it frames on one cable, through the
network's switch, onto the peer's cable. Everything is recorded through the
benchmark store beside the published-port and Redis lanes so the paths
compare on one report. Exploratory: no release baseline, no threshold, and
the doctor's verdict on the machine travels with the run.
"""

import hashlib
import json
import subprocess

import pytest
from helpers.constants import ASSETS_DIR, BIN_DIR, PROJECT_ROOT

from tests.ironbank.kingslanding.network import (
    address_of,
    bench_in,
    members,
    ping,
    serve_echo,
    udp,
)
from tests.ironbank.kingslanding.test_publish_benchmark import (
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
from tests.ironbank.kingslanding.test_run import service, wait_for

__all__ = ["evidence", "members", "service"]
pytestmark = pytest.mark.integration

REDIS_PORT = 6379
LANES = {
    "switch_tcp": "member container -> cable -> network switch -> cable -> member container",
    "switch_udp": "member container -> cable -> network switch -> cable -> member container, echoed back",
    "switch_ping": "member container -> cable -> network switch -> cable -> the peer's container, echoed back",
    "switch_redis": "member container Redis client -> cable -> network switch -> cable -> Redis in the peer's container",
}


def keep(output, identity, metrics, label, args, raw, settings_key):
    document = json.loads(raw)
    (output / f"{label}.json").write_text(raw)
    identity["trials"].append({"label": label, "args": args, "settings": document[settings_key]})
    lane = label.rsplit(".", 1)[0]
    for name, metric in document["metrics"].items():
        metrics.setdefault(f"{lane}.{name}", {"unit": metric["unit"], "samples": []})["samples"].extend(
            metric["samples"]
        )
    return document


def test_switch_transport_samples(members, service, evidence):
    alpha, beta, network = members["alpha"], members["beta"], members["network"]
    output = evidence
    doctor = subprocess.run([BENCH, "doctor", "--json"], capture_output=True, timeout=15, check=False)
    (output / "doctor.json").write_bytes(doctor.stdout)
    identity = {
        "doctor_exit": doctor.returncode,
        "source_commit": subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=PROJECT_ROOT, timeout=5, text=True
        ).strip(),
        "source_diff_sha256": hashlib.sha256(
            subprocess.check_output(["git", "diff", "HEAD"], cwd=PROJECT_ROOT, timeout=5)
        ).hexdigest(),
        "manifest": json.loads((ASSETS_DIR / "manifest.json").read_text()),
        "host_binaries": {
            name: hashlib.sha256((BIN_DIR / name).read_bytes()).hexdigest()
            for name in ("capsem-bench-rs", "capsem-process", "capsem-router", "capsem-service")
        },
        "lanes": LANES,
        "classification": "exploratory; no release baseline or performance threshold",
        "trials": [],
    }
    beta_address = address_of(service, network, beta)
    start_in_guest(
        service,
        beta["id"],
        "throughput-server",
        f"{IN_CONTAINER} capsem-bench-rs throughput --serve 0.0.0.0:{THROUGHPUT_PORT}",
    )
    serve_echo(service, beta)
    target = f"{beta_address}:{THROUGHPUT_PORT}"
    wait_for(
        lambda: bench_in(
            service, alpha, *client_args("latency", 1, seconds=1), "--address", target, timeout=15, check=False
        ).get("exit_code")
        == 0,
        "the peer's throughput server answers over the switch",
        timeout=60,
    )

    metrics = {}
    try:
        for direction, streams in MATRIX:
            for trial in range(TRIALS):
                args = client_args(direction, streams)
                raw = bench_in(service, alpha, *args, "--address", target)["stdout"]
                keep(output, identity, metrics, f"switch_tcp.{direction}.s{streams}.r{trial}", args, raw, "throughput")
        for _trial in range(TRIALS):
            udp(service, alpha, beta_address, 200, 1400, recorded=metrics, lane="switch_udp.b1400")
            ping(service, alpha, beta_address, 20, recorded=metrics, lane="switch_ping")
        for trial in range(TRIALS):
            args = ["redis", "--requests", "10000", "--concurrency", "32", "--pipeline", "16"]
            raw = bench_in(service, alpha, *args, "--address", f"{beta_address}:{REDIS_PORT}", timeout=60)["stdout"]
            document = keep(output, identity, metrics, f"switch_redis.c32.p16.r{trial}", args, raw, "redis")
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
