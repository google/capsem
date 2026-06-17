#!/usr/bin/env python3
"""Validate benchmark JSON artifacts and optionally draw latency/rps graphs."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

from pydantic import BaseModel, Field, ValidationError


class LoadLevel(BaseModel):
    concurrency: int = Field(gt=0)
    duration_s: float = Field(gt=0)
    total_requests: int = Field(ge=0)
    errors: int = Field(ge=0)
    rps: float = Field(ge=0)
    p50_ms: float = Field(ge=0)
    p95_ms: float = Field(ge=0)
    p99_ms: float = Field(ge=0)
    p999_ms: float = Field(ge=0)
    rss_peak_mb: float | None = Field(default=None, ge=0)


class LoadSeries(BaseModel):
    source: str
    name: str
    levels: list[LoadLevel]


LoadSeries.model_rebuild()


class LatencySummary(BaseModel):
    min: float = Field(ge=0)
    max: float = Field(ge=0)
    mean: float = Field(ge=0)
    p50: float = Field(ge=0)
    p95: float = Field(ge=0)
    p99: float = Field(ge=0)


class CountScenario(BaseModel):
    name: str
    total_requests: int = Field(gt=0)
    concurrency: int = Field(gt=0)
    successful: int = Field(ge=0)
    failed: int = Field(ge=0)
    requests_per_sec: float = Field(ge=0)
    latency_ms: LatencySummary


class CountSeries(BaseModel):
    source: str
    name: str
    scenarios: list[CountScenario]


CountSeries.model_rebuild()


def _load_json(path: Path) -> dict[str, Any]:
    with path.open() as handle:
        return json.load(handle)


def _extract_series(path: Path, data: dict[str, Any]) -> list[LoadSeries]:
    series = []
    for name in ("mitm_load", "mcp_load", "dns_load"):
        section = data.get(name)
        if isinstance(section, dict) and isinstance(
            section.get("concurrency_levels"), list
        ):
            series.append(
                LoadSeries(
                    source=str(path),
                    name=name,
                    levels=section["concurrency_levels"],
                )
            )

    # Direct artifact files under benchmarks/{mcp,dns,mitm}-load often have the
    # section itself at the document root.
    if not series and isinstance(data.get("concurrency_levels"), list):
        series.append(
            LoadSeries(
                source=str(path),
                name=path.parent.name.replace("-", "_"),
                levels=data["concurrency_levels"],
            )
        )
    return series


def _extract_count_series(path: Path, data: dict[str, Any]) -> list[CountSeries]:
    section = data.get("mock_server_protocol")
    if not isinstance(section, dict) or not isinstance(section.get("scenarios"), list):
        return []
    return [
        CountSeries(
            source=str(path),
            name="mock_server_protocol",
            scenarios=section["scenarios"],
        )
    ]


def load_series(paths: list[Path]) -> list[LoadSeries]:
    out = []
    errors = []
    for path in paths:
        try:
            out.extend(_extract_series(path, _load_json(path)))
        except (OSError, json.JSONDecodeError, ValidationError) as exc:
            errors.append(f"{path}: {exc}")
    if errors:
        raise SystemExit("\n".join(errors))
    return out


def load_count_series(paths: list[Path]) -> list[CountSeries]:
    out = []
    errors = []
    for path in paths:
        try:
            out.extend(_extract_count_series(path, _load_json(path)))
        except (OSError, json.JSONDecodeError, ValidationError) as exc:
            errors.append(f"{path}: {exc}")
    if errors:
        raise SystemExit("\n".join(errors))
    return out


def print_markdown(series: list[LoadSeries]) -> None:
    if not series:
        return
    print("| source | bench | c | requests | errors | rps | p50 ms | p95 ms | p99 ms | p999 ms |")
    print("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
    for item in series:
        for row in item.levels:
            print(
                f"| {item.source} | {item.name} | {row.concurrency} | "
                f"{row.total_requests} | {row.errors} | {row.rps:.1f} | "
                f"{row.p50_ms:.3f} | {row.p95_ms:.3f} | "
                f"{row.p99_ms:.3f} | {row.p999_ms:.3f} |"
            )


def print_count_markdown(series: list[CountSeries]) -> None:
    if not series:
        return
    print("| source | bench | scenario | c | sample_count | success | failed | error_rate | rps | p50 ms | p99 ms |")
    print("|---|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|")
    for item in series:
        for row in item.scenarios:
            error_rate = (row.failed / row.total_requests) * 100
            print(
                f"| {item.source} | {item.name} | {row.name} | {row.concurrency} | "
                f"{row.total_requests} | {row.successful}/{row.total_requests} | "
                f"{row.failed} | {error_rate:.3f}% | "
                f"{row.requests_per_sec:.1f} | {row.latency_ms.p50:.3f} | "
                f"{row.latency_ms.p99:.3f} |"
            )


def write_plot(
    load_series: list[LoadSeries],
    count_series: list[CountSeries],
    out_path: Path,
) -> None:
    try:
        import matplotlib.pyplot as plt
    except ImportError as exc:
        raise SystemExit(
            "matplotlib is required for --plot; run with "
            "`uv run --with matplotlib scripts/benchmark_report.py ... --plot out.png`"
        ) from exc

    fig, (ax_rps, ax_p99) = plt.subplots(1, 2, figsize=(12, 5), constrained_layout=True)
    for item in load_series:
        xs = [row.concurrency for row in item.levels]
        ax_rps.plot(xs, [row.rps for row in item.levels], marker="o", label=item.name)
        ax_p99.plot(
            xs,
            [row.p99_ms for row in item.levels],
            marker="o",
            label=item.name,
        )
    for item in count_series:
        xs = [row.name for row in item.scenarios]
        ax_rps.plot(
            xs,
            [row.requests_per_sec for row in item.scenarios],
            marker="o",
            label=item.name,
        )
        ax_p99.plot(
            xs,
            [row.latency_ms.p99 for row in item.scenarios],
            marker="o",
            label=item.name,
        )

    ax_rps.set_title("Throughput")
    ax_rps.set_xlabel("concurrency")
    ax_rps.set_ylabel("requests/sec")
    ax_rps.grid(True, alpha=0.3)
    ax_rps.legend()

    ax_p99.set_title("Tail latency")
    ax_p99.set_xlabel("concurrency")
    ax_p99.set_ylabel("p99 ms")
    ax_p99.grid(True, alpha=0.3)
    ax_p99.legend()

    out_path.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(out_path, dpi=160)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("artifacts", nargs="+", type=Path)
    parser.add_argument("--plot", type=Path, help="Write a PNG graph")
    args = parser.parse_args(argv)

    series = load_series(args.artifacts)
    count_series = load_count_series(args.artifacts)
    if not series and not count_series:
        raise SystemExit("no benchmark series found")
    print_markdown(series)
    print_count_markdown(count_series)
    if args.plot:
        write_plot(series, count_series, args.plot)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
