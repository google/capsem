from __future__ import annotations

import pytest

from tests.ironbank.test_route_health import (
    test_concurrent_route_reads_while_writes_are_active as _concurrent_read_write_gate,
    test_hot_control_routes_have_latency_and_cpu_budgets as _hot_route_latency_gate,
    test_seeded_session_ledger_routes_have_latency_and_cpu_budgets as _seeded_session_latency_gate,
)

pytestmark = [pytest.mark.integration, pytest.mark.serial]


def test_hot_control_routes_have_latency_and_cpu_budgets() -> None:
    _hot_route_latency_gate()


def test_concurrent_route_reads_while_writes_are_active() -> None:
    _concurrent_read_write_gate()


def test_seeded_session_ledger_routes_have_latency_and_cpu_budgets() -> None:
    _seeded_session_latency_gate()
