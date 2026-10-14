"""Raw route timing, throughput and per-process CPU series."""


def route_metrics(
    label: str,
    path: str,
    latencies: list[float],
    cold_latencies: list[float] | None,
    cpu: dict[str, float],
    rps: float | None = None,
) -> dict[str, dict]:
    concurrent = "concurrent_" if rps is not None else ""
    prefix = f"{label}.{path}."
    metrics = {
        f"{prefix}{concurrent}latency_ms": {"unit": "milliseconds", "samples": latencies},
    }
    if cold_latencies is not None:
        metrics[f"{prefix}cold_latency_ms"] = {"unit": "milliseconds", "samples": cold_latencies}
    if rps is not None:
        metrics[f"{prefix}concurrent_rps"] = {"unit": "requests_per_second", "samples": [rps]}
    for name, seconds in cpu.items():
        suffix = "cpu_s" if name == label else f"{name}_cpu_s"
        metrics[f"{prefix}{concurrent}{suffix}"] = {"unit": "seconds", "samples": [seconds]}
    return metrics
