"""Shared helpers for benchmark artifact naming and metadata."""

from __future__ import annotations

import copy
import os
import platform
import subprocess
import time
from datetime import datetime, timezone
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
    enriched["recorded_at_utc"] = datetime.now(timezone.utc).isoformat()
    run_id = os.environ.get("CAPSEM_BENCHMARK_RUN_ID", "").strip()
    if run_id:
        enriched["run_id"] = run_id
    if command:
        enriched["command"] = command
    enriched["host"] = _host_metadata()
    git_status = _git(project_root, "status", "--short", "--untracked-files=all")
    dirty_paths = _git_status_paths(git_status)
    source_dirty_paths = [
        path for path in dirty_paths
        if not (path == "benchmarks" or path.startswith("benchmarks/"))
    ]
    enriched["git"] = {
        "commit": _git(project_root, "rev-parse", "HEAD"),
        "dirty": bool(dirty_paths),
        "source_dirty": bool(source_dirty_paths),
        "dirty_paths": dirty_paths[:50],
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


def _git_status_paths(status: str) -> list[str]:
    if not status or status == "unknown":
        return []
    paths = []
    for line in status.splitlines():
        if len(line) < 4:
            continue
        path = line[3:].strip()
        if " -> " in path:
            path = path.rsplit(" -> ", 1)[1]
        if path:
            paths.append(path)
    return paths


def _host_metadata() -> dict[str, Any]:
    system = platform.system()
    host = {
        "platform": system,
        "release": platform.release(),
        "version": platform.version(),
        "machine": platform.machine(),
        "processor": platform.processor(),
        "python_version": platform.python_version(),
        "cpu_count": os.cpu_count() or 1,
        "cpu_count_logical": os.cpu_count() or 1,
    }
    if system == "Linux":
        host.update(_linux_host_metadata())
    elif system == "Darwin":
        host.update(_darwin_host_metadata())
    return host


def _linux_host_metadata() -> dict[str, Any]:
    info: dict[str, Any] = {}
    cpuinfo = _read_text(Path("/proc/cpuinfo"))
    if cpuinfo:
        model_names = []
        physical_ids = set()
        core_ids = set()
        for line in cpuinfo.splitlines():
            if ":" not in line:
                continue
            key, value = [part.strip() for part in line.split(":", 1)]
            if key == "model name" and value:
                model_names.append(value)
            elif key == "Hardware" and value:
                model_names.append(value)
            elif key == "physical id" and value:
                physical_ids.add(value)
            elif key == "core id" and value:
                core_ids.add(value)
        if model_names:
            info["cpu_model"] = model_names[0]
        if physical_ids and core_ids:
            info["cpu_count_physical"] = len(physical_ids) * len(core_ids)

    meminfo = _read_text(Path("/proc/meminfo"))
    if meminfo:
        for line in meminfo.splitlines():
            if line.startswith("MemTotal:"):
                kb = int(line.split()[1])
                info["memory_total_bytes"] = kb * 1024
                info["memory_total_gb"] = round((kb * 1024) / (1024**3), 2)
                break

    os_release = _read_os_release(Path("/etc/os-release"))
    if os_release:
        info["os_pretty_name"] = os_release.get("PRETTY_NAME")
        info["os_id"] = os_release.get("ID")
        info["os_version_id"] = os_release.get("VERSION_ID")
    return {k: v for k, v in info.items() if v not in (None, "")}


def _darwin_host_metadata() -> dict[str, Any]:
    info: dict[str, Any] = {}
    sysctls = {
        "cpu_model": "machdep.cpu.brand_string",
        "cpu_count_physical": "hw.physicalcpu",
        "cpu_count_logical": "hw.logicalcpu",
        "memory_total_bytes": "hw.memsize",
        "os_product_version": "kern.osproductversion",
    }
    for out_key, sysctl_key in sysctls.items():
        value = _sysctl(sysctl_key)
        if not value:
            continue
        if out_key in {"cpu_count_physical", "cpu_count_logical", "memory_total_bytes"}:
            try:
                info[out_key] = int(value)
            except ValueError:
                continue
        else:
            info[out_key] = value
    if "memory_total_bytes" in info:
        info["memory_total_gb"] = round(info["memory_total_bytes"] / (1024**3), 2)
    return info


def _read_text(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return ""


def _read_os_release(path: Path) -> dict[str, str]:
    data = {}
    text = _read_text(path)
    for line in text.splitlines():
        if "=" not in line or line.startswith("#"):
            continue
        key, value = line.split("=", 1)
        data[key] = value.strip().strip('"')
    return data


def _sysctl(key: str) -> str:
    try:
        return subprocess.check_output(
            ["sysctl", "-n", key],
            text=True,
            stderr=subprocess.DEVNULL,
        ).strip()
    except Exception:
        return ""
