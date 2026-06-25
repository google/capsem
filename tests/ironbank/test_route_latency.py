from __future__ import annotations

from tests.ironbank.test_route_health import (
    test_hot_control_routes_have_latency_and_cpu_budgets as _hot_route_latency_gate,
)


def test_hot_control_routes_have_latency_and_cpu_budgets() -> None:
    _hot_route_latency_gate()
