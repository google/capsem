"""Refresh eligible existing benchmark recordings, retaining reviewed data after artifact expiry."""

from __future__ import annotations

import io
import json
import re
import subprocess
import zipfile

from .. import crypto as hashlib
from .benchmarks import normalize
from .github import GitHub

PREFIXES = {
    "capsem-bench",
    "fork",
    "lifecycle",
    "mock-server-protocol",
    "route-latency",
    "parallel",
}


def snapshot_from_archive(data: bytes, artifact: dict) -> dict:
    records = {}
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        for entry in archive.infolist():
            parts = entry.filename.split("/")
            if not entry.filename.endswith(".json") or parts[0] not in PREFIXES:
                continue
            if ".." in parts or entry.file_size > 8 * 1024 * 1024:
                raise ValueError("Benchmark JSON archive member exceeds its permitted shape")
            records[entry.filename] = json.loads(archive.read(entry))
    names = [name for name in records if name.startswith("capsem-bench/")]
    if not names:
        raise ValueError("Benchmark artifact contains no supported regular guest recording")
    match = re.search(r"data_(\d+\.\d+\.\d+)_", names[0])
    if not match:
        raise ValueError("Benchmark artifact has no identifiable release filename")
    run = artifact["workflow_run"]
    result = {
        "source": {
            "artifact_id": artifact["id"],
            "release_label": match[1],
            "architecture": records[names[0]]["arch"],
            "source_commit": run["head_sha"],
            "artifact_created_at_utc": artifact["created_at"],
            "workflow_run_url": f"https://github.com/google/capsem/actions/runs/{run['id']}",
            "archive_sha256": hashlib.sha256(data).hexdigest(),
        },
        "records": records,
    }
    normalize(result)
    return result


def refresh(snapshot: dict, client: GitHub, repo: str = "google/capsem") -> dict:
    candidates = []
    for lane in ["binary", "runtime"]:
        for attempt in [1, 2, 3]:
            candidates.extend(
                client.pages(
                    f"repos/{repo}/actions/artifacts?name=hosted-benchmarks-{lane}-{attempt}",
                    "artifacts",
                )
            )
    candidates = [r for r in candidates if not r["expired"]]
    if not candidates:
        return snapshot
    latest = max(candidates, key=lambda r: r["created_at"])
    if latest["created_at"] <= snapshot["source"]["artifact_created_at_utc"]:
        return snapshot
    if latest["size_in_bytes"] > 64 * 1024 * 1024:
        raise ValueError("Benchmark artifact exceeds the read-only import size limit")
    response = subprocess.run(
        [
            "gh",
            "api",
            "--allow-escape-sequences",
            f"repos/{repo}/actions/artifacts/{latest['id']}/zip",
        ],
        capture_output=True,
        timeout=120,
        check=True,
    )
    return snapshot_from_archive(response.stdout, latest)
