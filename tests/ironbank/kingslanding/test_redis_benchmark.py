"""Exploratory Redis transport measurements, recorded by the benchmark owner.

Kingslanding uses a pinned native image prepared before hermetic execution.
"""

import hashlib
import json
import shlex
import subprocess
import tempfile
from pathlib import Path

import pytest
from helpers.benchmark_output import benchmark_output_dir
from helpers.constants import ASSETS_DIR, BIN_DIR, PROJECT_ROOT

from tests.ironbank.kingslanding.test_publish import redis, service

__all__ = ["redis", "service"]
pytestmark = pytest.mark.integration


def test_redis_guest_and_published_transport_samples(redis, service):
    output = Path(
        tempfile.mkdtemp(prefix="redis-", dir=benchmark_output_dir(PROJECT_ROOT, "kingslanding"))
    )
    bench = str(BIN_DIR / "capsem-bench-rs")
    doctor = subprocess.run(
        [bench, "doctor", "--json"], capture_output=True, timeout=15, check=False
    )
    (output / "doctor.json").write_bytes(doctor.stdout)
    (output / "doctor.stderr").write_bytes(doctor.stderr)
    identity = {
        "doctor_exit": doctor.returncode,
        "source_commit": subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=PROJECT_ROOT, timeout=5, text=True
        ).strip(),
        "source_diff_sha256": hashlib.sha256(
            subprocess.check_output(["git", "diff", "HEAD"], cwd=PROJECT_ROOT, timeout=5)
        ).hexdigest(),
        "manifest": json.loads((ASSETS_DIR / "manifest.json").read_text()),
        "image": redis["reference"],
        "host_binaries": {
            name: hashlib.sha256((BIN_DIR / name).read_bytes()).hexdigest()
            for name in ("capsem-bench-rs", "capsem-process", "capsem-port-router")
        },
        "classification": "exploratory; no release baseline or performance threshold",
        "trials": [],
    }
    metrics = {}
    for concurrency, pipeline in ((1, 1), (32, 1), (32, 16)):
        requests = 10000 if concurrency == 1 else 100000 * pipeline // (2 if pipeline > 1 else 1)
        for repetition in range(3):
            for lane in ("guest_local", "host_published"):
                args = [
                    "redis",
                    "--requests",
                    str(requests),
                    "--concurrency",
                    str(concurrency),
                    "--pipeline",
                    str(pipeline),
                ]
                label = f"{lane}.c{concurrency}.p{pipeline}.r{repetition}"
                if lane == "guest_local":
                    invocation = (
                        'nsenter -t "$(cat /var/tmp/capsem-container/workload.pid)" -n '
                        + shlex.join(["capsem-bench-rs", *args, "--address", "127.0.0.1:6379"])
                    )
                    response = service.client().post(
                        f"/vms/{redis['vm']['id']}/exec",
                        {"command": invocation, "timeout_secs": 40},
                        timeout=45,
                    )
                    assert response.get("exit_code") == 0, response
                    raw = response["stdout"]
                else:
                    invocation = [
                        bench,
                        *args,
                        "--address",
                        f"127.0.0.1:{redis['port']}",
                    ]
                    result = subprocess.run(
                        invocation,
                        capture_output=True,
                        text=True,
                        timeout=40,
                        check=False,
                    )
                    assert result.returncode == 0, result.stderr
                    raw = result.stdout
                document = json.loads(raw)
                assert document["metrics"]["completed_requests"]["samples"] == [requests]
                (output / f"{label}.json").write_text(raw)
                identity["trials"].append(
                    {
                        "label": label,
                        "command": invocation,
                        "settings": document["redis"],
                    }
                )
                for name, metric in document["metrics"].items():
                    key = f"{lane}.c{concurrency}.p{pipeline}.{name}"
                    metrics.setdefault(key, {"unit": metric["unit"], "samples": []})[
                        "samples"
                    ].extend(metric["samples"])
    (output / "identity.json").write_text(json.dumps(identity, indent=2) + "\n")
    collectors = output / "collectors"
    collectors.mkdir()
    (collectors / "vsock").write_text(json.dumps({"metrics": metrics}))
    store = output / "benchmarks.db"
    result = subprocess.run(
        [
            bench,
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
            identity["source_commit"],
        ],
        capture_output=True,
        timeout=30,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    (output / "record.json").write_bytes(result.stdout)
    report = subprocess.run(
        [bench, "report", "--store", str(store)],
        capture_output=True,
        timeout=10,
        check=False,
    )
    assert report.returncode == 0, report.stderr
    (output / "report.txt").write_bytes(report.stdout)
