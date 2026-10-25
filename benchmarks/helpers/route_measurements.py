"""Raw route observations, throughput and per-process CPU; no statistics."""
from __future__ import annotations

from dataclasses import dataclass, field

from helpers.http_measurement import MeasuredResponse


def validate_receipt(receipt: MeasuredResponse) -> int:
    first = receipt.first_body_byte_ns
    if receipt.attempts != 1 or first is None or not receipt.body or not 0 <= receipt.headers_ns <= first <= receipt.complete_ns:
        raise ValueError("route requires a complete single-attempt body receipt")
    return first


@dataclass
class Observations:
    completion: list[float] = field(default_factory=list)
    headers: list[float] = field(default_factory=list)
    first_body_byte: list[float] = field(default_factory=list)

    def append(self, receipt: MeasuredResponse) -> None:
        first = validate_receipt(receipt)
        self.completion.append(receipt.complete_ns / 1_000_000)
        self.headers.append(receipt.headers_ns / 1_000_000)
        self.first_body_byte.append(first / 1_000_000)

    def extend(self, other: Observations) -> None:
        self.completion.extend(other.completion)
        self.headers.extend(other.headers)
        self.first_body_byte.extend(other.first_body_byte)

    def require_count(self, expected: int) -> None:
        if expected <= 0 or any(len(series) != expected for series in (self.completion, self.headers, self.first_body_byte)):
            raise ValueError("route observation count differs from requested samples")

    def metrics(self, prefix: str, expected: int) -> dict[str, dict]:
        self.require_count(expected)
        return {prefix + key: {"unit": "milliseconds", "samples": series} for key, series in (
            ("latency_ms", self.completion), ("headers_ms", self.headers),
            ("first_body_byte_ms", self.first_body_byte),
        )}


def route_metrics(
    label: str, path: str, latencies: Observations,
    cold_latencies: Observations | None, cpu: dict[str, float],
    rps: float | None = None, *, expected: int,
) -> dict[str, dict]:
    concurrent = "concurrent_" if rps is not None else ""
    prefix = f"{label}.{path}."
    metrics = latencies.metrics(prefix + concurrent, expected)
    if cold_latencies is not None:
        metrics.update(cold_latencies.metrics(prefix + "cold_", expected))
    if rps is not None:
        metrics[f"{prefix}concurrent_rps"] = {"unit": "requests_per_second", "samples": [rps]}
    for name, seconds in cpu.items():
        suffix = "cpu_s" if name == label else f"{name}_cpu_s"
        metrics[f"{prefix}{concurrent}{suffix}"] = {"unit": "seconds", "samples": [seconds]}
    return metrics
