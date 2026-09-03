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
    assert budget.cpu_s == 0.5
    assert_has_headroom(
        label="service /stats during profile-mutation writes CPU",
        measured=0.378,
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


def test_non_finite_measurements_are_rejected() -> None:
    with pytest.raises(ValidationError):
        HeadroomGuard(
            label="route",
            measured=float("nan"),
            ceiling=5.0,
            minimum_factor=1.2,
            unit="ms",
        )


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
