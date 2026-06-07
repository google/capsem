#!/usr/bin/env python3
"""Archive Criterion db_writer_pressure output as release benchmark JSON."""

import argparse
import json
import os
import re
import time
from pathlib import Path


DEFAULT_CRITERION_DIR = Path("target/criterion/db_writer_pressure")


def project_version(root: Path) -> str:
    cargo = root / "Cargo.toml"
    match = re.search(r'^version\s*=\s*"([^"]+)"', cargo.read_text(), re.MULTILINE)
    return match.group(1) if match else "unknown"


def load_json(path: Path):
    with path.open() as handle:
        return json.load(handle)


def estimate_ms(estimates: dict, key: str) -> float:
    value_ns = estimates[key]["point_estimate"]
    return round(value_ns / 1_000_000.0, 4)


def confidence_ms(estimates: dict, key: str) -> dict:
    interval = estimates[key]["confidence_interval"]
    return {
        "confidence_level": interval["confidence_level"],
        "lower_ms": round(interval["lower_bound"] / 1_000_000.0, 4),
        "upper_ms": round(interval["upper_bound"] / 1_000_000.0, 4),
    }


def percentile(values: list[float], pct: float) -> float:
    values = sorted(values)
    if not values:
        return 0.0
    position = (len(values) - 1) * pct / 100.0
    lower = int(position)
    upper = min(lower + 1, len(values) - 1)
    if lower == upper:
        return values[lower]
    weight = position - lower
    return values[lower] * (1.0 - weight) + values[upper] * weight


def sample_percentiles(path: Path) -> dict:
    sample_path = path / "sample.json"
    if not sample_path.exists():
        return {}
    sample = load_json(sample_path)
    latencies_ms = [
        (float(total_ns) / float(iters)) / 1_000_000.0
        for total_ns, iters in zip(sample.get("times", []), sample.get("iters", []))
        if float(iters) > 0
    ]
    return {
        "p50_ms": round(percentile(latencies_ms, 50), 4),
        "p95_ms": round(percentile(latencies_ms, 95), 4),
        "p99_ms": round(percentile(latencies_ms, 99), 4),
    }


def parse_burst_dir(path: Path) -> dict:
    benchmark = load_json(path / "benchmark.json")
    estimates = load_json(path / "estimates.json")
    burst_size = int(benchmark["throughput"]["Elements"])
    mean_ms = estimate_ms(estimates, "mean")
    median_ms = estimate_ms(estimates, "median")
    return {
        "name": benchmark["function_id"],
        "burst_size": burst_size,
        "mean_ms": mean_ms,
        "median_ms": median_ms,
        "events_per_sec_mean": round(burst_size / (mean_ms / 1000.0), 1),
        "events_per_sec_median": round(burst_size / (median_ms / 1000.0), 1),
        "sample_percentiles": sample_percentiles(path),
        "mean_confidence": confidence_ms(estimates, "mean"),
        "median_confidence": confidence_ms(estimates, "median"),
    }


def collect_db_writer_benchmark(criterion_dir: Path) -> dict:
    rows = []
    for burst_dir in sorted(criterion_dir.glob("file_events_*/new")):
        rows.append(parse_burst_dir(burst_dir))
    if not rows:
        raise FileNotFoundError(
            f"no Criterion db_writer_pressure results found under {criterion_dir}; "
            "run `cargo bench -p capsem-logger --bench db_writer_pressure -- --quiet`"
        )
    return {
        "version": "1.0",
        "benchmark": "db_writer_pressure",
        "source": str(criterion_dir),
        "rows": rows,
    }


def archive(root: Path, data: dict) -> Path:
    version = project_version(root)
    arch = "arm64" if os.uname().machine == "arm64" else "x86_64"
    out_dir = root / "benchmarks" / "db-writer"
    out_dir.mkdir(parents=True, exist_ok=True)
    out_path = out_dir / f"data_{version}_{arch}.json"
    payload = {
        **data,
        "project_version": version,
        "arch": os.uname().machine,
        "host_recorded_at": time.time(),
        "notes": (
            "Criterion benchmark of the real capsem_logger::DbWriter writing "
            "file-event bursts to SQLite and shutting down cleanly."
        ),
    }
    with out_path.open("w") as handle:
        json.dump(payload, handle, indent=2)
        handle.write("\n")
    return out_path


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path.cwd())
    parser.add_argument("--criterion-dir", type=Path, default=DEFAULT_CRITERION_DIR)
    args = parser.parse_args()
    root = args.root.resolve()
    criterion_dir = args.criterion_dir
    if not criterion_dir.is_absolute():
        criterion_dir = root / criterion_dir
    data = collect_db_writer_benchmark(criterion_dir)
    out_path = archive(root, data)
    print(out_path)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
