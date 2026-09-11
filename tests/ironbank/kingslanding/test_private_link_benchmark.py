"""Exploratory private-link measurements: the tun0 lane against the FD-pair lane.

The same `capsem-bench-rs throughput` client runs on both sides of Capsem.
`host_published` is today's data plane: a host client, a published port, the
confined router copying bytes into the container. `guest_tun` is the proposed
one: a guest client, tun0, capsem-tun pumping packets over VSOCK, smoltcp in
the VM owner terminating them. Kingslanding uses a pinned native image
prepared before hermetic execution; the image only provides the container.
"""

import hashlib
import json
import re
import shlex
import subprocess
import tempfile
from pathlib import Path

import pytest
from helpers.benchmark_output import benchmark_output_dir
from helpers.constants import ASSETS_DIR, BIN_DIR, PROJECT_ROOT

from tests.fixtures.oci.registry import registry
from tests.ironbank.kingslanding.test_run import command, environment, service, wait_for

__all__ = ["service"]
pytestmark = pytest.mark.integration

THROUGHPUT_PORT = 5201
GATEWAY = "10.128.0.1"
SECONDS = 3
TRIALS = 3
MATRIX = [
    (direction, streams)
    for direction in ("upload", "download", "bidirectional")
    for streams in (1, 4, 16)
]
MATRIX.append(("latency", 1))
IN_CONTAINER = 'nsenter -t "$(cat /var/tmp/capsem-container/workload.pid)" -n'
BENCH = str(BIN_DIR / "capsem-bench-rs")


@pytest.fixture
def container(service, tmp_path):
    """The Redis image as a container with one published port; the throughput
    server runs in its network namespace from the guest binary."""
    with (
        registry(tmp_path) as (reference, certificate, _),
        (tmp_path / "stdout").open("wb") as stdout,
        (tmp_path / "stderr").open("wb") as stderr,
    ):
        process = subprocess.Popen(
            command(service, reference, certificate, "-p", f"0:{THROUGHPUT_PORT}"),
            env=environment(service),
            stdout=stdout,
            stderr=stderr,
        )
        try:

            def ready():
                assert process.poll() is None, (tmp_path / "stderr").read_text()
                return (
                    b"Ready to accept connections tcp"
                    in (tmp_path / "stdout").read_bytes()
                )

            wait_for(ready, "container startup")
            mappings = re.findall(
                r"Published 127.0.0.1:(\d+) -> (\d+)/tcp",
                (tmp_path / "stderr").read_text(),
            )
            assert len(mappings) == 1, mappings
            rows = service.client().get("/vms/list")["sandboxes"]
            assert len(rows) == 1
            yield {"port": int(mappings[0][0]), "vm": rows[0], "reference": reference}
        finally:
            for log in service.tmp_dir.glob("persistent/*/process.log"):
                (tmp_path / "process.log").write_bytes(log.read_bytes())
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=30)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
            for row in service.client().get("/vms/list")["sandboxes"]:
                service.client().delete(f"/vms/{row['id']}/delete")


def guest(service, vm_id, shell, timeout=40, check=True):
    response = service.client().post(
        f"/vms/{vm_id}/exec",
        {"command": shell, "timeout_secs": timeout},
        timeout=timeout + 5,
    )
    if check:
        assert response.get("exit_code") == 0, response
    return response


def start_in_guest(service, vm_id, name, shell):
    """Detach a long-lived helper; its log comes back with the evidence."""
    detached = (
        f"setsid sh -c {shlex.quote(shell)} </dev/null >/var/tmp/{name}.log 2>&1 &"
    )
    guest(service, vm_id, detached)


def probe(argv):
    return subprocess.run(argv, capture_output=True, text=True, timeout=30, check=False)


def client_args(direction, streams, seconds=SECONDS):
    return [
        "throughput",
        "--direction",
        direction,
        "--streams",
        str(streams),
        "--seconds",
        str(seconds),
    ]


def test_private_link_and_published_port_transport_samples(
    container, service, tmp_path
):
    output = Path(
        tempfile.mkdtemp(
            prefix="private-link-",
            dir=benchmark_output_dir(PROJECT_ROOT, "kingslanding"),
        )
    )
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
        "image": container["reference"],
        "host_binaries": {
            name: hashlib.sha256((BIN_DIR / name).read_bytes()).hexdigest()
            for name in ("capsem-bench-rs", "capsem-process", "capsem-router")
        },
        "lanes": {
            "host_published": "host client -> published port -> capsem-router -> VSOCK -> container",
            "guest_tun": "guest client -> tun0 -> capsem-tun -> VSOCK -> smoltcp in capsem-process",
        },
        "classification": "exploratory; no release baseline or performance threshold",
        "trials": [],
    }
    vm_id = container["vm"]["id"]

    # The far end of each lane, then a latency probe proves it answers.
    start_in_guest(
        service,
        vm_id,
        "throughput-server",
        f"{IN_CONTAINER} capsem-bench-rs throughput --serve 0.0.0.0:{THROUGHPUT_PORT}",
    )
    published = f"127.0.0.1:{container['port']}"
    wait_for(
        lambda: (
            probe(
                [
                    BENCH,
                    *client_args("latency", 1, seconds=1),
                    "--address",
                    published,
                ]
            ).returncode
            == 0
        ),
        "published throughput server",
        timeout=30,
    )
    start_in_guest(
        service,
        vm_id,
        "capsem-tun",
        # The VM's lifetime address comes from the service, never guessed here.
        f"capsem-tun --address {container['vm']['private_address']} --peer {GATEWAY}",
    )
    gateway = f"{GATEWAY}:{THROUGHPUT_PORT}"
    tun_probe = shlex.join(
        [
            "capsem-bench-rs",
            *client_args("latency", 1, seconds=1),
            "--address",
            gateway,
        ]
    )
    wait_for(
        lambda: (
            guest(service, vm_id, tun_probe, timeout=15, check=False).get("exit_code")
            == 0
        ),
        "tun0 lane answers",
        timeout=30,
    )

    metrics = {}
    try:
        measure_lanes(service, vm_id, published, gateway, output, identity, metrics)
    finally:
        # The helpers' logs are the evidence when a lane never answers.
        for name in ("capsem-tun", "throughput-server"):
            log = guest(service, vm_id, f"cat /var/tmp/{name}.log", check=False)
            (output / f"{name}.log").write_text(
                log.get("stdout", "") + log.get("stderr", "")
            )
        helpers = guest(
            service,
            vm_id,
            "ls -la /var/tmp; pgrep -a capsem-tun; ip -o addr show tun0",
            check=False,
        )
        (output / "guest-helpers.txt").write_text(json.dumps(helpers, indent=2))
        print(f"PRIVATE LINK EVIDENCE: {output}")
    (output / "identity.json").write_text(json.dumps(identity, indent=2) + "\n")
    (output / "comparison.json").write_text(
        json.dumps(compare(metrics), indent=2) + "\n"
    )
    record(output, metrics, identity["source_commit"])


def measure_lanes(service, vm_id, published, gateway, output, identity, metrics):
    lanes = (
        ("host_published", lambda args: probe([BENCH, *args, "--address", published])),
        (
            "guest_tun",
            lambda args: guest(
                service,
                vm_id,
                shlex.join(["capsem-bench-rs", *args, "--address", gateway]),
            ),
        ),
    )
    for lane, run in lanes:
        for direction, streams in MATRIX:
            for trial in range(TRIALS):
                args = client_args(direction, streams)
                result = run(args)
                if isinstance(result, subprocess.CompletedProcess):
                    assert result.returncode == 0, result.stderr
                    raw = result.stdout
                else:
                    raw = result["stdout"]
                document = json.loads(raw)
                label = f"{lane}.{direction}.s{streams}.r{trial}"
                (output / f"{label}.json").write_text(raw)
                identity["trials"].append(
                    {"label": label, "args": args, "settings": document["throughput"]}
                )
                for name, metric in document["metrics"].items():
                    key = f"{lane}.{direction}.s{streams}.{name}"
                    metrics.setdefault(key, {"unit": metric["unit"], "samples": []})[
                        "samples"
                    ].extend(metric["samples"])


def record(output, metrics, source_commit):
    """The same store and report the Redis benchmark uses."""
    collectors = output / "collectors"
    collectors.mkdir()
    (collectors / "vsock").write_text(json.dumps({"metrics": metrics}))
    store = output / "benchmarks.db"
    recorded = subprocess.run(
        [
            BENCH,
            "run",
            "vsock",
            "--collectors",
            str(collectors),
            "--interpreter",
            "/bin/cat",
            "--out",
            str(store),
            "--channel",
            "spike",
            "--commit",
            source_commit,
        ],
        capture_output=True,
        timeout=30,
        check=False,
    )
    assert recorded.returncode == 0, recorded.stderr
    (output / "record.json").write_bytes(recorded.stdout)
    report = subprocess.run(
        [BENCH, "report", "--store", str(store)],
        capture_output=True,
        timeout=10,
        check=False,
    )
    assert report.returncode == 0, report.stderr
    (output / "report.txt").write_bytes(report.stdout)


def compare(metrics):
    """Per cell, the tun0 lane as a fraction of the FD-pair lane: throughput
    on the direction's moving side, median round trip for latency."""

    def mean(key):
        samples = metrics[key]["samples"]
        return sum(samples) / len(samples)

    def median(key):
        samples = sorted(metrics[key]["samples"])
        return samples[len(samples) // 2]

    cells = {}
    for direction, streams in MATRIX:
        cell = f"{direction}.s{streams}"
        if f"guest_tun.{cell}.elapsed_seconds" not in metrics:
            continue
        if direction == "latency":
            published = median(f"host_published.{cell}.round_trip_ms")
            tun = median(f"guest_tun.{cell}.round_trip_ms")
            cells[cell] = {
                "published_median_ms": published,
                "tun_median_ms": tun,
                "tun_over_published": tun / published,
            }
            continue
        sides = {
            "upload": ["send"],
            "download": ["receive"],
            "bidirectional": ["send", "receive"],
        }[direction]
        for side in sides:
            published = mean(f"host_published.{cell}.{side}_megabits_per_sec")
            tun = mean(f"guest_tun.{cell}.{side}_megabits_per_sec")
            cells[f"{cell}.{side}"] = {
                "published_mbit_s": published,
                "tun_mbit_s": tun,
                "tun_over_published": tun / published if published else None,
            }
    return {
        "decision_rule": "proceed if the tun0 lane keeps roughly a third of the FD-pair throughput",
        "cells": cells,
    }
