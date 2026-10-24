"""Raw clocks/outcomes only; capsem-bench owns statistics and fitness."""

from __future__ import annotations


def metrics(report: dict, *, host_wall_ns: int) -> dict:
    request = report["original_request"]
    elapsed = request["elapsed_ns"]
    activation = report["activation_verified_elapsed_ns"]
    outcomes = (
        "succeeded",
        "transport_failure",
        "http_failure",
        "application_failure",
        "invalid_response",
    )
    if (
        report.get("activation") != "verified"
        or request.get("transport") not in {"service_uds", "gateway_http"}
        or request.get("outcome") not in outcomes
        or any(type(value) is not int or value < 0 for value in (elapsed, activation, host_wall_ns))
        or activation < elapsed
    ):
        raise ValueError("Install receipt is unverified or has an impossible clock/outcome")
    result = {
        "original_request.time_to_terminal_ms": {
            "unit": "milliseconds",
            "samples": [elapsed / 1000000],
        },
        "activation.verified_ms": {"unit": "milliseconds", "samples": [activation / 1000000]},
        "owner.host_wall_seconds": {"unit": "seconds", "samples": [host_wall_ns / 1000000000]},
    }
    for outcome in outcomes:
        result[f"original_request.{outcome}_count"] = {
            "unit": "count",
            "samples": [int(request["outcome"] == outcome)],
        }
    return result
