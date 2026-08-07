"""Benchmark output routing for clean gates and explicit archival runs."""

from __future__ import annotations

import os
from pathlib import Path


def benchmark_output_dir(project_root: Path, category: str) -> Path:
    configured = os.environ.get("CAPSEM_BENCHMARK_OUTPUT_ROOT")
    root = Path(configured) if configured else project_root / "target" / "test-benchmarks"
    output = root / category
    output.mkdir(parents=True, exist_ok=True)
    return output
