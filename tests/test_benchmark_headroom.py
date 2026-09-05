"""The performance gate keeps operating margin, not merely a green inequality."""

from __future__ import annotations

import pytest
from helpers.benchmark_ratchet import HeadroomGuard, assert_has_headroom
from helpers.route_health_budget import concurrent_stats_budget, scaled_hot_route_budget
from pydantic import ValidationError


def test_exact_minimum_headroom_passes() -> None:
    assert_has_headroom(
        label="gateway /vms/list p95",
        measured=4.0,
        ceiling=4.8,
        minimum_factor=1.2,
        unit="ms",
    )


def test_consumed_headroom_fails_with_the_required_ceiling() -> None:
    with pytest.raises(AssertionError, match=r"required_ceiling=4\.920ms"):
        assert_has_headroom(
            label="gateway /vms/list p95",
            measured=4.1,
            ceiling=4.8,
            minimum_factor=1.2,
            unit="ms",
        )


def test_accounting_tick_is_part_of_the_cpu_ceiling() -> None:
    assert_has_headroom(
        label="service /vms/list CPU",
        measured=0.132,
        ceiling=0.15,
        minimum_factor=1.2,
        accounting_slack=0.011,
        unit="s",
    )


def test_concurrent_stats_budget_has_measured_twenty_percent_headroom() -> None:
    budget = concurrent_stats_budget()

    assert budget.p95_ms == 15.0
    assert budget.p99_ms == 40.0
    assert_has_headroom(
        label="service /stats during profile-mutation writes CPU",
        measured=0.442,
        ceiling=budget.cpu_s,
        minimum_factor=1.2,
        accounting_slack=0.011,
        unit="s",
    )


def test_vm_scalar_gateway_budget_has_measured_twenty_percent_headroom() -> None:
    budget = scaled_hot_route_budget(
        "/vms/33333333-3333-4333-8333-333333333333/info",
        gateway=True,
        samples=128,
    )

    assert budget.p95_ms == 6.0
    assert_has_headroom(
        label="gateway /vms/{id}/info p95",
        measured=4.574,
        ceiling=budget.p95_ms,
        minimum_factor=1.2,
        unit="ms",
    )


def test_evaluate_gateway_cpu_has_measured_twenty_percent_headroom() -> None:
    budget = scaled_hot_route_budget(
        "/profiles/code/enforcement/evaluate",
        gateway=True,
        samples=128,
    )

    assert budget.cpu_s == 0.40
    assert_has_headroom(
        label="gateway /profiles/code/enforcement/evaluate CPU",
        measured=0.316,
        ceiling=budget.cpu_s,
        minimum_factor=1.2,
        accounting_slack=0.011,
        unit="s",
    )


@pytest.mark.parametrize(
    ("owner", "measured_ms"), [("detection", 1.767), ("enforcement", 1.905)]
)
def test_evaluate_service_latency_has_measured_twenty_percent_headroom(
    owner: str, measured_ms: float
) -> None:
    # Full qualification and its focused reproduction on the same fit Linux
    # host crossed the old margin on different aliases of the same handler.
    path = f"/profiles/code/{owner}/evaluate"
    budget = scaled_hot_route_budget(path, gateway=False, samples=128)
    assert_has_headroom(
        label=f"service {path} p95",
        measured=measured_ms,
        ceiling=budget.p95_ms,
        minimum_factor=1.2,
        unit="ms",
    )
    assert budget.p95_ms == 3.0


def test_non_finite_measurements_are_rejected() -> None:
    with pytest.raises(ValidationError):
        HeadroomGuard(
            label="route",
            measured=float("nan"),
            ceiling=5.0,
            minimum_factor=1.2,
            unit="ms",
        )


def test_route_ceilings_use_human_time_increments() -> None:
    with pytest.raises(ValidationError, match="5ms increment"):
        from capsem_builder.gate.benchmarkschema import RouteBudgetConfig

        RouteBudgetConfig(p95_ms=16.0, p99_ms=45.0, cpu_s=0.5)


def test_route_ceiling_scales_cpu_but_not_latency() -> None:
    budget = scaled_hot_route_budget(
        "/profiles/code/detection/rules/list",
        gateway=True,
        samples=128,
    )

    assert budget.p95_ms == 5.0
    assert budget.p99_ms == 8.0
    assert budget.cpu_s == 0.30


def test_richer_gateway_inventory_has_a_rounded_six_ms_ceiling() -> None:
    budget = scaled_hot_route_budget(
        "/profiles/status",
        gateway=True,
        samples=128,
    )

    assert budget.p95_ms == 6.0
    assert budget.cpu_s == 0.36
