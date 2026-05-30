#!/usr/bin/env python3
"""Compare committed Linux and macOS benchmark artifacts."""

from __future__ import annotations

import argparse
import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any


PROJECT_ROOT = Path(__file__).resolve().parent.parent


@dataclass(frozen=True)
class Metric:
    label: str
    category: str
    path: tuple[str, ...]
    unit: str
    better: str
    suffix: str | None = None
    delta_kind: str = "latency"


METRICS: tuple[Metric, ...] = (
    Metric("Scratch seq write", "capsem-bench", ("disk", "seq_write", "throughput_mbps"), "MB/s", "higher"),
    Metric("Scratch seq read", "capsem-bench", ("disk", "seq_read", "throughput_mbps"), "MB/s", "higher"),
    Metric("Scratch rand write", "capsem-bench", ("disk", "rand_write_4k", "iops"), "IOPS", "higher"),
    Metric("Scratch rand read", "capsem-bench", ("disk", "rand_read_4k", "iops"), "IOPS", "higher"),
    Metric("Rootfs seq read", "capsem-bench", ("rootfs", "seq_read", "throughput_mbps"), "MB/s", "higher"),
    Metric("Rootfs rand read", "capsem-bench", ("rootfs", "rand_read_4k", "iops"), "IOPS", "higher"),
    Metric("Rootfs large binary cold", "capsem-bench", ("rootfs", "large_binary_seq_read", "cold_throughput_mbps"), "MB/s", "higher"),
    Metric("Rootfs small JS reads", "capsem-bench", ("rootfs", "small_js_read", "ops_per_sec"), "ops/s", "higher"),
    Metric("Rootfs metadata stat", "capsem-bench", ("rootfs", "metadata_stat", "stats_per_sec"), "stats/s", "higher"),
    Metric("Startup python3", "capsem-bench", ("startup", "commands", "python3", "mean_ms"), "ms", "lower"),
    Metric("Startup node", "capsem-bench", ("startup", "commands", "node", "mean_ms"), "ms", "lower"),
    Metric("Startup claude", "capsem-bench", ("startup", "commands", "claude", "mean_ms"), "ms", "lower"),
    Metric("Startup gemini", "capsem-bench", ("startup", "commands", "gemini", "mean_ms"), "ms", "lower"),
    Metric("Startup codex", "capsem-bench", ("startup", "commands", "codex", "mean_ms"), "ms", "lower"),
    Metric("Lifecycle provision", "lifecycle", ("operations", "provision_ms", "mean"), "ms", "lower"),
    Metric("Lifecycle exec ready", "lifecycle", ("operations", "exec_ready_ms", "mean"), "ms", "lower"),
    Metric("Lifecycle exec", "lifecycle", ("operations", "exec_ms", "mean"), "ms", "lower"),
    Metric("Lifecycle delete", "lifecycle", ("operations", "delete_ms", "mean"), "ms", "lower"),
    Metric("Lifecycle total", "lifecycle", ("operations", "total_ms", "mean"), "ms", "lower"),
    Metric("Fork create", "fork", ("fork", "fork_ms", "mean"), "ms", "lower"),
    Metric("Fork image size", "fork", ("fork", "image_size_mb", "mean"), "MB", "lower", delta_kind="size"),
    Metric("Fork boot provision", "fork", ("fork", "boot_provision_ms", "mean"), "ms", "lower"),
    Metric("Fork boot ready", "fork", ("fork", "boot_ready_ms", "mean"), "ms", "lower"),
    Metric("Security process block", "security-engine", ("operations", "blocked_process_exec_ms", "mean"), "ms", "lower", "process_enforcement"),
    Metric("Security HTTP block wall", "security-engine", ("operations", "blocked_http_request_wall_ms", "mean"), "ms", "lower", "http_request_enforcement"),
    Metric("Security HTTP keepalive", "security-engine", ("operations", "keepalive_http_request_total_ms", "mean"), "ms", "lower", "http_request_enforcement"),
    Metric("Security DNS block", "security-engine", ("operations", "blocked_dns_request_ms", "mean"), "ms", "lower", "dns_request_enforcement"),
    Metric("Security MCP block", "security-engine", ("operations", "blocked_mcp_request_ms", "mean"), "ms", "lower", "mcp_request_enforcement"),
)

EXPECTED_LANES: tuple[tuple[str, str], ...] = (
    ("capsem-bench", "in-VM disk/rootfs/startup/HTTP/throughput/snapshot"),
    ("lifecycle", "VM lifecycle"),
    ("fork", "fork and boot-from-image"),
    ("host-native", "host-native baseline"),
    ("security-engine/*_enforcement", "VM-originated Security Engine"),
    ("security-engine/*_microbench", "Criterion Security Engine"),
)


def artifact_pattern(category: str, arch: str, suffix: str | None) -> str:
    if suffix:
        return f"data_*_{arch}_{suffix}.json"
    return f"data_*_{arch}.json"


def latest_artifact(root: Path, category: str, arch: str, suffix: str | None = None) -> Path | None:
    directory = root / "benchmarks" / category
    if not directory.exists():
        return None

    matches = sorted(directory.glob(artifact_pattern(category, arch, suffix)))
    if matches:
        return matches[-1]

    # Older macOS lifecycle/fork artifacts predate arch-scoped filenames.
    if arch == "arm64" and suffix is None and category in {"lifecycle", "fork"}:
        legacy = [
            path
            for path in sorted(directory.glob("data_*.json"))
            if not path.stem.endswith(("_arm64", "_x86_64"))
        ]
        if legacy:
            return legacy[-1]

    return None


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text())


def read_path(data: dict[str, Any], path: tuple[str, ...]) -> float | None:
    value: Any = data
    for key in path:
        if not isinstance(value, dict) or key not in value:
            return None
        value = value[key]
    if isinstance(value, int | float):
        return float(value)
    return None


def compare_values(linux: float, mac: float, better: str, delta_kind: str = "latency") -> tuple[float, str]:
    ratio = linux / mac if mac else float("inf")
    if better == "higher":
        if ratio >= 1:
            return ratio, f"{(ratio - 1) * 100:.1f}% higher"
        return ratio, f"{(1 - ratio) * 100:.1f}% lower"
    if delta_kind == "size":
        if ratio <= 1:
            return ratio, f"{(1 - ratio) * 100:.1f}% smaller"
        return ratio, f"{(ratio - 1) * 100:.1f}% larger"
    if ratio <= 1:
        return ratio, f"{(1 - ratio) * 100:.1f}% faster"
    return ratio, f"{(ratio - 1) * 100:.1f}% slower"


def format_value(value: float, unit: str) -> str:
    if unit in {"IOPS", "ops/s", "stats/s"}:
        return f"{value:,.0f} {unit}"
    if value >= 100:
        return f"{value:,.1f} {unit}"
    return f"{value:.3f} {unit}"


def collect_rows(root: Path, linux_arch: str, mac_arch: str) -> tuple[list[dict[str, str]], list[str]]:
    rows: list[dict[str, str]] = []
    missing: list[str] = []
    cache: dict[tuple[str, str, str | None], dict[str, Any] | None] = {}

    for metric in METRICS:
        linux_key = (metric.category, linux_arch, metric.suffix)
        mac_key = (metric.category, mac_arch, metric.suffix)
        if linux_key not in cache:
            path = latest_artifact(root, metric.category, linux_arch, metric.suffix)
            cache[linux_key] = load_json(path) if path else None
        if mac_key not in cache:
            path = latest_artifact(root, metric.category, mac_arch, metric.suffix)
            cache[mac_key] = load_json(path) if path else None

        linux_data = cache[linux_key]
        mac_data = cache[mac_key]
        if linux_data is None or mac_data is None:
            missing.append(f"{metric.label}: missing artifact")
            continue

        linux_value = read_path(linux_data, metric.path)
        mac_value = read_path(mac_data, metric.path)
        if linux_value is None or mac_value is None:
            missing.append(f"{metric.label}: missing metric")
            continue

        ratio, status = compare_values(linux_value, mac_value, metric.better, metric.delta_kind)
        rows.append(
            {
                "metric": metric.label,
                "linux": format_value(linux_value, metric.unit),
                "mac": format_value(mac_value, metric.unit),
                "ratio": f"{ratio:.2f}x",
                "status": status,
            }
        )

    missing.extend(missing_lanes(root, linux_arch, mac_arch))
    return rows, missing


def missing_lanes(root: Path, linux_arch: str, mac_arch: str) -> list[str]:
    missing = []
    checks = [
        ("host-native", None),
        ("security-engine", "cel_microbench"),
        ("security-engine", "security_packs_microbench"),
    ]
    for category, suffix in checks:
        linux = latest_artifact(root, category, linux_arch, suffix)
        mac = latest_artifact(root, category, mac_arch, suffix)
        if linux is not None and mac is None:
            label = f"{category}/{suffix}" if suffix else category
            missing.append(f"{label}: missing {mac_arch} artifact")
    return missing


def render_markdown(rows: list[dict[str, str]], missing: list[str]) -> str:
    lines = [
        "| Metric | Linux x86_64 | macOS arm64 | Linux/Mac | Linux status |",
        "|--------|--------------|-------------|-----------|--------------|",
    ]
    for row in rows:
        lines.append(
            f"| {row['metric']} | {row['linux']} | {row['mac']} | {row['ratio']} | {row['status']} |"
        )
    if missing:
        lines.append("")
        lines.append("Missing comparison lanes:")
        for item in missing:
            lines.append(f"- {item}")
    return "\n".join(lines)


def render_json(rows: list[dict[str, str]], missing: list[str]) -> str:
    return json.dumps({"rows": rows, "missing": missing}, indent=2)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=PROJECT_ROOT)
    parser.add_argument("--linux-arch", default="x86_64")
    parser.add_argument("--mac-arch", default="arm64")
    parser.add_argument("--format", choices=("markdown", "json"), default="markdown")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    rows, missing = collect_rows(args.root, args.linux_arch, args.mac_arch)
    if args.format == "json":
        print(render_json(rows, missing))
    else:
        print(render_markdown(rows, missing))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
