"""Typed route-health ceilings and the minimum-margin guard."""

from __future__ import annotations

from pathlib import Path
from typing import Literal, Protocol, cast

from capsem_builder.gate.benchmarkschema import RouteBudgetPairConfig
from capsem_builder.gate.config import for_root
from pydantic import BaseModel, ConfigDict, Field

from helpers.benchmark_ratchet import assert_has_headroom

PROJECT_ROOT = Path(__file__).resolve().parents[2]
SETTINGS = for_root(PROJECT_ROOT).benchmark.route_health
CPU_ACCOUNTING_SLACK_S = float(SETTINGS.cpu_accounting_slack_s)
HOT_ROUTE_REFERENCE_SAMPLES = int(SETTINGS.reference_samples)
HOT_ROUTE_WINDOW_SAMPLES = int(SETTINGS.window_samples)
HOT_ROUTE_WINDOWS = int(SETTINGS.windows)
HOT_ROUTE_HEADROOM_FACTOR = float(SETTINGS.minimum_headroom_factor)
BudgetName = Literal[
    "aggregate_ledger",
    "assets_status",
    "default",
    "evaluate",
    "latest",
    "ledger",
    "mcp_default",
    "mcp_servers",
    "plugin_info",
    "profiles",
    "rules",
    "stats",
    "stats_detail",
    "status",
    "vm_scalar",
    "vms_list",
]


class Timing(Protocol):
    @property
    def label(self) -> str: ...

    @property
    def samples_ms(self) -> list[float]: ...

    @property
    def service_cpu_s(self) -> float: ...

    @property
    def gateway_cpu_s(self) -> float | None: ...

    @property
    def p50_ms(self) -> float: ...

    @property
    def p95_ms(self) -> float: ...

    @property
    def p99_ms(self) -> float: ...

    @property
    def max_ms(self) -> float: ...


class RouteBudget(BaseModel):
    """A route's actual ceilings before sample-count scaling."""

    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    p95_ms: float = Field(gt=0, allow_inf_nan=False)
    p99_ms: float = Field(gt=0, allow_inf_nan=False)
    cpu_s: float = Field(gt=0, allow_inf_nan=False)

    def for_samples(self, samples: int) -> RouteBudget:
        return self.model_copy(
            update={"cpu_s": self.cpu_s * samples / HOT_ROUTE_REFERENCE_SAMPLES}
        )


def assert_timing_budget(
    timing: Timing,
    *,
    p95_ms: float,
    max_ms: float | None,
    cpu_s: float,
    p99_ms: float | None = None,
    cpu_slack_s: float = CPU_ACCOUNTING_SLACK_S,
    headroom_factor: float = HOT_ROUTE_HEADROOM_FACTOR,
) -> None:
    """Require every measurement to retain the configured operating margin."""
    print(
        "ROUTE_HEALTH "
        f"{timing.label} p50={timing.p50_ms:.1f}ms "
        f"p95={timing.p95_ms:.1f}ms "
        f"p99={timing.p99_ms:.1f}ms max={timing.max_ms:.1f}ms "
        f"service_cpu={timing.service_cpu_s:.3f}s "
        f"gateway_cpu={timing.gateway_cpu_s if timing.gateway_cpu_s is not None else 'n/a'}"
    )
    for label, measured, ceiling in (
        ("p95", timing.p95_ms, p95_ms),
        ("p99", timing.p99_ms, p99_ms),
        ("max", timing.max_ms, max_ms),
    ):
        if ceiling is not None:
            assert_has_headroom(
                label=f"{timing.label} {label}",
                measured=measured,
                ceiling=ceiling,
                minimum_factor=headroom_factor,
                unit="ms",
            )
    assert_has_headroom(
        label=f"{timing.label} service CPU",
        measured=timing.service_cpu_s,
        ceiling=cpu_s,
        minimum_factor=headroom_factor,
        accounting_slack=cpu_slack_s,
        unit="s",
    )
    if timing.gateway_cpu_s is not None:
        assert_has_headroom(
            label=f"{timing.label} gateway CPU",
            measured=timing.gateway_cpu_s,
            ceiling=cpu_s,
            minimum_factor=headroom_factor,
            accounting_slack=cpu_slack_s,
            unit="s",
        )


def _is_vm_scalar_state_route(path: str) -> bool:
    if "/vms/" not in path:
        return False
    suffix = path.split("/vms/", 1)[1].split("?", 1)[0]
    return suffix.count("/") == 1 and suffix.endswith(("/status", "/info"))


def _budget(name: BudgetName, *, gateway: bool) -> RouteBudget:
    pair = cast(RouteBudgetPairConfig, getattr(SETTINGS.budgets, name))
    configured = pair.gateway if gateway else pair.service
    return RouteBudget(
        p95_ms=float(configured.p95_ms),
        p99_ms=float(configured.p99_ms),
        cpu_s=float(configured.cpu_s),
    )


def hot_route_budget(path: str, *, gateway: bool = False) -> RouteBudget:
    """Resolve one route to rounded latency and CPU ceilings."""
    budget_name: BudgetName = "default"
    if path == "/status":
        budget_name = "status"
    if _is_vm_scalar_state_route(path):
        budget_name = "vm_scalar"
    elif path == "/vms/list":
        budget_name = "vms_list"
    elif path in {"/profiles/list", "/profiles/status"}:
        budget_name = "profiles"
    elif "/stats/detail" in path:
        budget_name = "stats_detail"
    elif any(
        marker in path
        for marker in (
            "/history",
            "/timeline",
            "/security/status",
            "/security/latest",
            "/detection/status",
            "/detection/latest",
            "/enforcement/status",
            "/enforcement/latest",
        )
    ):
        aggregate = any(
            path.endswith(marker) or f"{marker}?" in path
            for marker in (
                "/security/status",
                "/security/latest",
                "/detection/status",
                "/detection/latest",
                "/enforcement/status",
                "/enforcement/latest",
            )
        )
        budget_name = "aggregate_ledger" if aggregate else "ledger"
    elif path.endswith("/assets/status"):
        budget_name = "assets_status"
    elif path.endswith("/mcp/default/info"):
        budget_name = "mcp_default"
    elif path.endswith("/mcp/servers/list"):
        budget_name = "mcp_servers"
    elif path.endswith("/rules/list"):
        budget_name = "rules"
    elif path.endswith("/latest"):
        budget_name = "latest"
    elif path.endswith("/evaluate"):
        budget_name = "evaluate"
    elif "/plugins/" in path and path.endswith(("/info", "/credentials/info")):
        budget_name = "plugin_info"
    elif path == "/stats":
        budget_name = "stats"
    return _budget(budget_name, gateway=gateway)


def scaled_hot_route_budget(
    path: str,
    *,
    gateway: bool,
    samples: int,
) -> RouteBudget:
    return hot_route_budget(path, gateway=gateway).for_samples(samples)


def assert_hot_route_budget(
    timing: Timing,
    *,
    path: str,
    gateway: bool = False,
) -> None:
    assert len(timing.samples_ms) == HOT_ROUTE_WINDOW_SAMPLES
    budget = scaled_hot_route_budget(
        path,
        gateway=gateway,
        samples=HOT_ROUTE_WINDOW_SAMPLES,
    )
    assert_timing_budget(
        timing,
        p95_ms=budget.p95_ms,
        p99_ms=budget.p99_ms,
        max_ms=None,
        cpu_s=budget.cpu_s,
    )
