"""Exploratory published-port measurements on the FD-pair data plane.

A host client, a published port, the confined router copying bytes into the
container: `capsem-bench-rs throughput` records every direction and stream
count through the benchmark store, beside the member-to-member lanes of
`test_network_benchmark.py`. The shared helpers here (a guest exec, a
detached helper, the store) serve every kingslanding network test.
Kingslanding uses a pinned native image prepared before hermetic execution;
the image only provides the container.
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
def evidence():
    """Where a test's evidence outlives the run: the gate removes its
    temporary directories, so a log copied to tmp_path was gone before
    anyone could read it."""
    return Path(
        tempfile.mkdtemp(
            prefix="kingslanding-",
            dir=benchmark_output_dir(PROJECT_ROOT, "kingslanding"),
        )
    )


@pytest.fixture
def container(service, tmp_path, evidence):
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
            # The VM owner's log and the guest's serial console: when every
            # VSOCK link to the guest ends at once, the console is the only
            # witness on the guest side.
            for name in ("process.log", "serial.log", ".capsem-agent-stdio.log"):
                for log in service.tmp_dir.rglob(name):
                    (evidence / name).write_bytes(log.read_bytes())
            print(f"KINGSLANDING EVIDENCE: {evidence}")
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


def unplugged(service, vm_id):
    """Whether the guest has no cable and no pump running: a VM in no network."""
    links = guest(service, vm_id, "ip -o link show; pgrep -x capsem-tun || true")
    return " cable" not in links["stdout"] and not any(
        line.strip().isdigit() for line in links["stdout"].splitlines()
    )


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


def test_published_port_transport_samples(
    container, service, evidence
):
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
        "image": container["reference"],
        "host_binaries": {
            name: hashlib.sha256((BIN_DIR / name).read_bytes()).hexdigest()
            for name in ("capsem-bench-rs", "capsem-process", "capsem-router")
        },
        "lanes": {
            "host_published": "host client -> published port -> capsem-router -> VSOCK -> container",
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
    # Every VM starts unplugged: no network, so no cable.
    assert unplugged(service, vm_id)

    metrics = {}
    try:
        measure_lane(published, output, identity, metrics)
    finally:
        # The helpers' logs are the evidence when the lane never answers.
        for name, command in (
            ("throughput-server", "cat /var/tmp/throughput-server.log"),
        ):
            log = guest(service, vm_id, command, check=False)
            # The whole response: an exec that never answered is itself the
            # evidence, and an empty log with no exit code said nothing.
            (output / f"{name}.log").write_text(
                log.get("stdout", "") + log.get("stderr", "")
                if log.get("exit_code") == 0
                else json.dumps(log, indent=2)
            )
        helpers = guest(
            service,
            vm_id,
            "ls -la /var/tmp; pgrep -a capsem-tun; ip -o link",
            check=False,
        )
        (output / "guest-helpers.txt").write_text(json.dumps(helpers, indent=2))
    (output / "identity.json").write_text(json.dumps(identity, indent=2) + "\n")
    record(output, metrics, identity["source_commit"])


def measure_lane(published, output, identity, metrics):
    lane = "host_published"
    for direction, streams in MATRIX:
        for trial in range(TRIALS):
            args = client_args(direction, streams)
            result = probe([BENCH, *args, "--address", published])
            assert result.returncode == 0, result.stderr
            raw = result.stdout
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
