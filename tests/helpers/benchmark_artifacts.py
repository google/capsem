"""Shared helpers for benchmark artifact naming and metadata."""

from __future__ import annotations

import copy
import os
import platform
import subprocess
import time
from pathlib import Path
from typing import Any


SCHEMA = "capsem.benchmark-artifact.v1"


def benchmark_arch() -> str:
    machine = platform.machine().lower()
    if machine in {"aarch64", "arm64"}:
        return "arm64"
    if machine in {"x86_64", "amd64"}:
        return "x86_64"
    return machine or "unknown"


def benchmark_output_path(
    project_root: Path,
    category: str,
    project_version: str,
    arch: str | None = None,
) -> Path:
    output_root = Path(
        os.environ.get("CAPSEM_BENCHMARK_OUTPUT_DIR", project_root / "benchmarks")
    )
    out_dir = output_root / category
    run_id = os.environ.get("CAPSEM_BENCHMARK_RUN_ID", "").strip()
    parts = ["data", project_version, arch or benchmark_arch()]
    if run_id:
        parts.append(run_id)
    return out_dir / ("_".join(parts) + ".json")


def enrich_benchmark_artifact(
    data: dict[str, Any],
    *,
    project_root: Path,
    project_version: str,
    arch: str | None = None,
    command: str | None = None,
) -> dict[str, Any]:
    enriched = copy.deepcopy(data)
    enriched.setdefault("schema", SCHEMA)
    enriched["project_version"] = project_version
    enriched["arch"] = arch or benchmark_arch()
    enriched["recorded_at"] = time.time()
    run_id = os.environ.get("CAPSEM_BENCHMARK_RUN_ID", "").strip()
    if run_id:
        enriched["run_id"] = run_id
    if command:
        enriched["command"] = command
    enriched["host"] = {
        "platform": platform.system(),
        "release": platform.release(),
        "machine": platform.machine(),
        "cpu_count": os.cpu_count() or 1,
    }
    enriched["git"] = {
        "commit": _git(project_root, "rev-parse", "HEAD"),
        "dirty": bool(_git(project_root, "status", "--short")),
    }
    return enriched


def _git(project_root: Path, *args: str) -> str:
    try:
        return subprocess.check_output(
            ["git", *args],
            cwd=project_root,
            text=True,
            stderr=subprocess.DEVNULL,
        ).strip()
    except Exception:
        return "unknown"
