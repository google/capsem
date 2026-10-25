"""Pure measurement and quality helpers for the shared guest collector."""

from __future__ import annotations

import json

UNITS = {
    "_mbps": "megabits_per_second",
    "_ms": "milliseconds",
    "_bytes": "bytes",
    "iops": "operations",
    "rps": "requests_per_second",
}


def failed_load_levels(section: dict) -> list[int]:
    """Return concurrency levels where any request failed."""
    return [
        int(row.get("concurrency", -1))
        for row in section.get("concurrency_levels", [])
        if int(row.get("errors", 0)) > 0
    ]


def mitm_runtime_failures(service_log: str) -> list[str]:
    """Return trusted-path traffic refusals that an HTTP client may retry."""
    failures = []
    for line in service_log.splitlines():
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            continue
        fields = record.get("fields", {})
        if fields.get("message") == "proxy worker refused traffic descriptor":
            failures.append(
                str(fields.get("error", "proxy worker refused traffic descriptor"))
            )
    return failures


def resource_samples(metrics: dict) -> dict[str, float]:
    """Flatten sampled host pressure for diagnostics from rejected load runs."""
    return {
        name: measurement["samples"][0]
        for name, measurement in sorted(metrics.items())
        if measurement.get("samples")
    }


def unit_for(key: str) -> str | None:
    """Return the unit declared by a measurement-key suffix."""
    for suffix, unit in UNITS.items():
        if key.endswith(suffix):
            return unit
    return None


def flatten(node, prefix: str = "") -> dict[str, float]:
    """Return every numeric leaf under its dotted key."""
    found: dict[str, float] = {}
    if isinstance(node, dict):
        for key, value in node.items():
            found.update(flatten(value, f"{prefix}.{key}" if prefix else str(key)))
    elif isinstance(node, list):
        for index, value in enumerate(node):
            found.update(flatten(value, f"{prefix}.{index}" if prefix else str(index)))
    elif not isinstance(node, bool) and isinstance(node, (int, float)):
        found[prefix] = float(node)
    return found


def load_quality_metrics(section: dict) -> dict[str, dict]:
    """Store zero-error proof for every load concurrency level."""
    metrics = {}
    for index, row in enumerate(section.get("concurrency_levels", [])):
        errors = int(row.get("errors", 0))
        total = int(row.get("total_requests", 0))
        metrics[f"concurrency_levels.{index}.errors"] = {
            "unit": "count",
            "samples": [errors],
        }
        metrics[f"concurrency_levels.{index}.error_ratio"] = {
            "unit": "ratio",
            "samples": [errors / total if total else 0.0],
        }
        metrics[f"concurrency_levels.{index}.requests"] = {
            "unit": "count",
            "samples": [total],
        }
    return metrics
