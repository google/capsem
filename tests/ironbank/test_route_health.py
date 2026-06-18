"""Ironbank route health gates for Capsem control surfaces.

These tests are intentionally black-box. They start the real service and gateway
processes, call the public routes the UI/TUI depend on, and fail when a route
quietly regresses into CPU-bound work such as hashing VM assets on a poll path.
"""

from __future__ import annotations

from dataclasses import dataclass
import statistics
import time
from typing import Any, Callable

import psutil
import pytest

from helpers.constants import (
    CODE_PROFILE_ID,
    DEFAULT_CPUS,
    DEFAULT_RAM_MB,
    EXEC_READY_TIMEOUT,
    EXEC_TIMEOUT_SECS,
    HTTP_TIMEOUT,
)
from helpers.gateway import GatewayInstance, TcpHttpClient
from helpers.service import ServiceInstance, wait_exec_ready, vm_name


pytestmark = pytest.mark.integration


@dataclass(frozen=True)
class RouteContract:
    method: str
    path: str
    body: dict[str, Any] | None
    required_keys: set[str] | None
    response_kind: type


@dataclass(frozen=True)
class RouteTiming:
    label: str
    samples_ms: list[float]
    service_cpu_s: float
    gateway_cpu_s: float | None

    @property
    def p50_ms(self) -> float:
        return statistics.median(self.samples_ms)

    @property
    def p95_ms(self) -> float:
        ordered = sorted(self.samples_ms)
        index = min(len(ordered) - 1, int(round((len(ordered) - 1) * 0.95)))
        return ordered[index]

    @property
    def max_ms(self) -> float:
        return max(self.samples_ms)


def _enforcement_payload(action: str = "block") -> dict[str, Any]:
    return {
        "rules_toml": f"""
[profiles.rules.route_health_{action}]
name = "route_health_{action}"
action = "{action}"
detection_level = "high"
match = 'http.host == "route-health.example"'
""".strip(),
        "event": {
            "event_type": "http.request",
            "http_host": "route-health.example",
        },
    }


def _call(client: Any, contract: RouteContract, *, timeout: int = 20) -> Any:
    if contract.method == "GET":
        return client.get(contract.path, timeout=timeout)
    if contract.method == "POST":
        return client.post(contract.path, contract.body, timeout=timeout)
    raise AssertionError(f"unsupported route method in health gate: {contract.method}")


def _assert_contract(client: Any, contract: RouteContract) -> None:
    payload = _call(client, contract)
    assert isinstance(payload, contract.response_kind), (contract.path, payload)
    if contract.required_keys is not None:
        assert contract.required_keys <= set(payload), (contract.path, payload)


def _assert_evaluation_decision(client: Any, *, profile: str, action: str) -> None:
    payload = client.post(
        f"/profiles/{profile}/enforcement/evaluate",
        _enforcement_payload(action),
        timeout=20,
    )
    assert set(payload) == {"event"}
    event = payload["event"]
    assert event["event_type"] == "http.request"
    assert event["http"]["host"] == "route-health.example"
    assert event["decision"] == {"effective": action}

    detections = event["detections"]
    assert len(detections) == 1
    assert detections[0] == {
        "source": "rule",
        "detection_level": "high",
        "rule_id": f"profiles.rules.route_health_{action}",
        "plugin_id": None,
        "action": action,
        "plugin_mode": None,
        "reason": None,
    }

    plugin_executions = event["plugin_executions"]
    assert [plugin["plugin_id"] for plugin in plugin_executions] == [
        "credential_broker",
        "log_sanitizer",
    ]
    assert [plugin["stage"] for plugin in plugin_executions] == [
        "preprocess",
        "logging",
    ]
    assert all(isinstance(plugin["duration_us"], int) for plugin in plugin_executions)


def _cpu_seconds(proc: psutil.Process) -> float:
    try:
        times = proc.cpu_times()
    except psutil.Error as error:  # pragma: no cover - test infra failure path
        raise AssertionError(f"unable to read CPU times for pid {proc.pid}: {error}") from error
    return float(times.user + times.system)


def _measure_route(
    label: str,
    call: Callable[[], Any],
    *,
    service_proc: psutil.Process,
    gateway_proc: psutil.Process | None = None,
    samples: int = 8,
) -> RouteTiming:
    for _ in range(2):
        call()
    service_before = _cpu_seconds(service_proc)
    gateway_before = _cpu_seconds(gateway_proc) if gateway_proc is not None else None
    timings: list[float] = []
    for _ in range(samples):
        started = time.perf_counter()
        call()
        timings.append((time.perf_counter() - started) * 1000.0)
    service_after = _cpu_seconds(service_proc)
    gateway_after = _cpu_seconds(gateway_proc) if gateway_proc is not None else None
    return RouteTiming(
        label=label,
        samples_ms=timings,
        service_cpu_s=service_after - service_before,
        gateway_cpu_s=(
            None
            if gateway_before is None or gateway_after is None
            else gateway_after - gateway_before
        ),
    )


def _measure_once(
    label: str,
    call: Callable[[], Any],
    *,
    service_proc: psutil.Process,
    gateway_proc: psutil.Process | None = None,
) -> tuple[Any, RouteTiming]:
    service_before = _cpu_seconds(service_proc)
    gateway_before = _cpu_seconds(gateway_proc) if gateway_proc is not None else None
    started = time.perf_counter()
    payload = call()
    elapsed_ms = (time.perf_counter() - started) * 1000.0
    service_after = _cpu_seconds(service_proc)
    gateway_after = _cpu_seconds(gateway_proc) if gateway_proc is not None else None
    return payload, RouteTiming(
        label=label,
        samples_ms=[elapsed_ms],
        service_cpu_s=service_after - service_before,
        gateway_cpu_s=(
            None
            if gateway_before is None or gateway_after is None
            else gateway_after - gateway_before
        ),
    )


def _assert_timing_budget(timing: RouteTiming, *, p95_ms: float, max_ms: float, cpu_s: float) -> None:
    print(
        "ROUTE_HEALTH "
        f"{timing.label} p50={timing.p50_ms:.1f}ms "
        f"p95={timing.p95_ms:.1f}ms max={timing.max_ms:.1f}ms "
        f"service_cpu={timing.service_cpu_s:.3f}s "
        f"gateway_cpu={timing.gateway_cpu_s if timing.gateway_cpu_s is not None else 'n/a'}"
    )
    assert timing.p95_ms <= p95_ms, (
        f"{timing.label} p95={timing.p95_ms:.1f}ms > {p95_ms}ms; "
        f"samples={timing.samples_ms}"
    )
    assert timing.max_ms <= max_ms, (
        f"{timing.label} max={timing.max_ms:.1f}ms > {max_ms}ms; "
        f"samples={timing.samples_ms}"
    )
    assert timing.service_cpu_s <= cpu_s, (
        f"{timing.label} service CPU={timing.service_cpu_s:.3f}s > {cpu_s:.3f}s"
    )
    if timing.gateway_cpu_s is not None:
        assert timing.gateway_cpu_s <= cpu_s, (
            f"{timing.label} gateway CPU={timing.gateway_cpu_s:.3f}s > {cpu_s:.3f}s"
        )


def _assert_vm_row(
    listing: dict[str, Any],
    vm_id: str,
    *,
    status: str | None = None,
    persistent: bool | None = None,
) -> dict[str, Any]:
    rows = listing["sandboxes"]
    row = next((candidate for candidate in rows if candidate["id"] == vm_id), None)
    assert row is not None, f"{vm_id} missing from /vms/list: {rows}"
    if status is not None:
        assert row["status"] == status, row
    if persistent is not None:
        assert row["persistent"] is persistent, row
    return row


def _assert_vm_absent(listing: dict[str, Any], vm_id: str) -> None:
    rows = listing["sandboxes"]
    assert vm_id not in {row["id"] for row in rows}, rows


def _service_route_contracts() -> list[RouteContract]:
    profile = CODE_PROFILE_ID
    return [
        RouteContract("GET", "/status", None, {"components", "ready", "service", "version"}, dict),
        RouteContract("GET", "/version", None, {"version"}, dict),
        RouteContract("GET", "/vms/list", None, {"sandboxes", "asset_health"}, dict),
        RouteContract("POST", "/purge", {}, {"purged", "persistent_purged", "ephemeral_purged"}, dict),
        RouteContract("GET", "/profiles/list", None, {"profiles"}, dict),
        RouteContract(
            "GET",
            "/profiles/status",
            None,
            {"asset_manifest", "profile_count", "profiles", "ready_count", "source"},
            dict,
        ),
        RouteContract("GET", f"/profiles/{profile}/info", None, {"profile", "obom"}, dict),
        RouteContract(
            "GET",
            f"/profiles/{profile}/assets/status",
            None,
            {"profile_id", "ready", "assets", "missing_assets", "invalid_assets", "manifest"},
            dict,
        ),
        RouteContract(
            "GET",
            f"/profiles/{profile}/assets/info",
            None,
            {"profile_id", "current_arch", "refresh_policy", "current_assets"},
            dict,
        ),
        RouteContract(
            "GET",
            f"/profiles/{profile}/enforcement/info",
            None,
            {"profile_id", "rule_count", "action_counts"},
            dict,
        ),
        RouteContract(
            "GET",
            f"/profiles/{profile}/enforcement/rules/list",
            None,
            {"profile_id", "rules"},
            dict,
        ),
        RouteContract(
            "POST",
            f"/profiles/{profile}/enforcement/evaluate",
            _enforcement_payload("block"),
            {"event"},
            dict,
        ),
        RouteContract(
            "POST",
            f"/profiles/{profile}/enforcement/evaluate",
            _enforcement_payload("ask"),
            {"event"},
            dict,
        ),
        RouteContract(
            "GET",
            f"/profiles/{profile}/detection/info",
            None,
            {"profile_id", "rule_count", "detection_rule_count"},
            dict,
        ),
        RouteContract(
            "GET",
            f"/profiles/{profile}/detection/rules/list",
            None,
            {"profile_id", "rules"},
            dict,
        ),
        RouteContract(
            "POST",
            f"/profiles/{profile}/detection/evaluate",
            _enforcement_payload("allow"),
            {"event"},
            dict,
        ),
        RouteContract("GET", f"/profiles/{profile}/plugins/list", None, {"scope", "plugins"}, dict),
        RouteContract(
            "GET",
            f"/profiles/{profile}/plugins/info",
            None,
            {"scope", "plugin_count", "enabled_count"},
            dict,
        ),
        RouteContract(
            "GET",
            f"/profiles/{profile}/plugins/credential_broker/credentials/info",
            None,
            {"scope", "plugin_id", "store", "inventory", "grants", "corp_constraints"},
            dict,
        ),
        RouteContract(
            "GET",
            f"/profiles/{profile}/mcp/info",
            None,
            {"profile_id", "server_count", "builtin_local_enabled"},
            dict,
        ),
        RouteContract(
            "GET",
            f"/profiles/{profile}/mcp/default/info",
            None,
            {"action", "source", "rule_id"},
            dict,
        ),
        RouteContract("GET", f"/profiles/{profile}/mcp/servers/list", None, None, list),
        RouteContract("GET", f"/profiles/{profile}/mcp/servers/local/tools/list", None, None, list),
        RouteContract("GET", "/settings/info", None, {"tree", "issues"}, dict),
        RouteContract("GET", "/corp/info", None, {"installed", "paths", "source"}, dict),
        RouteContract("GET", "/security/status", None, {"sessions", "total"}, dict),
        RouteContract("GET", "/security/latest", None, None, list),
        RouteContract("GET", "/enforcement/status", None, {"sessions", "total"}, dict),
        RouteContract("GET", "/enforcement/latest", None, None, list),
        RouteContract("GET", "/detection/status", None, {"sessions", "total"}, dict),
        RouteContract("GET", "/detection/latest", None, None, list),
        RouteContract("GET", "/stats", None, {"global", "sessions"}, dict),
    ]


def test_control_route_contracts_exist_for_ui_tui_blocking_and_vm_surfaces() -> None:
    service = ServiceInstance()
    try:
        service.start()
        client = service.client()
        for contract in _service_route_contracts():
            _assert_contract(client, contract)
        for action in ("allow", "ask", "block"):
            _assert_evaluation_decision(client, profile=CODE_PROFILE_ID, action=action)
    finally:
        service.stop()


def test_hot_control_routes_have_latency_and_cpu_budgets() -> None:
    service = ServiceInstance()
    gateway: GatewayInstance | None = None
    try:
        service.start()
        gateway = GatewayInstance(uds_path=service.uds_path)
        gateway.start()
        service_client = service.client()
        gateway_client = TcpHttpClient(gateway.base_url, gateway.token)
        assert service.proc is not None
        assert gateway.proc is not None
        service_proc = psutil.Process(service.proc.pid)
        gateway_proc = psutil.Process(gateway.proc.pid)

        hot_service_routes = [
            RouteContract("GET", "/status", None, {"ready", "service"}, dict),
            RouteContract("GET", "/vms/list", None, {"sandboxes"}, dict),
            RouteContract("GET", "/profiles/list", None, {"profiles"}, dict),
            RouteContract(
                "GET",
                "/profiles/status",
                None,
                {"profile_count", "profiles", "ready_count"},
                dict,
            ),
            RouteContract(
                "GET",
                f"/profiles/{CODE_PROFILE_ID}/plugins/list",
                None,
                {"scope", "plugins"},
                dict,
            ),
            RouteContract(
                "GET",
                f"/profiles/{CODE_PROFILE_ID}/enforcement/rules/list",
                None,
                {"profile_id", "rules"},
                dict,
            ),
        ]
        for contract in hot_service_routes:
            timing = _measure_route(
                f"service {contract.path}",
                lambda c=contract: _assert_contract(service_client, c),
                service_proc=service_proc,
            )
            _assert_timing_budget(timing, p95_ms=150.0, max_ms=250.0, cpu_s=0.20)

        hot_gateway_routes = [
            RouteContract(
                "GET",
                "/status",
                None,
                {"gateway_version", "service", "vm_count", "assets", "profiles"},
                dict,
            ),
            RouteContract("GET", "/vms/list", None, {"sandboxes"}, dict),
            RouteContract("GET", "/profiles/list", None, {"profiles"}, dict),
            RouteContract(
                "GET",
                "/profiles/status",
                None,
                {"profile_count", "profiles", "ready_count"},
                dict,
            ),
        ]
        for contract in hot_gateway_routes:
            timing = _measure_route(
                f"gateway {contract.path}",
                lambda c=contract: _assert_contract(gateway_client, c),
                service_proc=service_proc,
                gateway_proc=gateway_proc,
            )
            _assert_timing_budget(timing, p95_ms=250.0, max_ms=350.0, cpu_s=0.25)
    finally:
        if gateway is not None:
            gateway.stop()
        service.stop()


@pytest.mark.serial
def test_vm_session_lifecycle_routes_have_state_and_latency_budgets() -> None:
    service = ServiceInstance()
    gateway: GatewayInstance | None = None
    source_id = vm_name("ironbank-route-life")
    child_id = vm_name("ironbank-route-child")
    try:
        service.start()
        gateway = GatewayInstance(uds_path=service.uds_path)
        gateway.start()
        service_client = service.client()
        gateway_client = TcpHttpClient(gateway.base_url, gateway.token)
        assert service.proc is not None
        assert gateway.proc is not None
        service_proc = psutil.Process(service.proc.pid)
        gateway_proc = psutil.Process(gateway.proc.pid)

        create, timing = _measure_once(
            "service /vms/create persistent",
            lambda: service_client.post(
                "/vms/create",
                {
                    "name": source_id,
                    "profile_id": CODE_PROFILE_ID,
                    "ram_mb": DEFAULT_RAM_MB,
                    "cpus": DEFAULT_CPUS,
                    "persistent": True,
                },
                timeout=HTTP_TIMEOUT,
            ),
            service_proc=service_proc,
        )
        assert create["id"] == source_id
        assert create["profile_id"] == CODE_PROFILE_ID
        _assert_timing_budget(timing, p95_ms=45_000.0, max_ms=45_000.0, cpu_s=10.0)
        assert wait_exec_ready(service_client, source_id, timeout=EXEC_READY_TIMEOUT)

        for client_label, client, gateway_for_cpu in (
            ("service", service_client, None),
            ("gateway", gateway_client, gateway_proc),
        ):
            for contract in (
                RouteContract("GET", f"/vms/{source_id}/status", None, {"id", "status"}, dict),
                RouteContract("GET", f"/vms/{source_id}/info", None, {"id", "status"}, dict),
                RouteContract("GET", "/vms/list", None, {"sandboxes", "asset_health"}, dict),
            ):
                timing = _measure_route(
                    f"{client_label} {contract.path}",
                    lambda c=contract, route_client=client: _assert_contract(route_client, c),
                    service_proc=service_proc,
                    gateway_proc=gateway_for_cpu,
                )
                _assert_timing_budget(timing, p95_ms=350.0, max_ms=500.0, cpu_s=0.40)

        running_status = service_client.get(f"/vms/{source_id}/status", timeout=30)
        assert running_status["id"] == source_id
        assert running_status["status"] == "Running"
        assert running_status["persistent"] is True
        assert running_status["can_resume"] is False
        assert running_status["available_actions"] == ["pause", "stop", "fork", "delete"]
        running_info = service_client.get(f"/vms/{source_id}/info", timeout=30)
        assert running_info["profile_id"] == CODE_PROFILE_ID
        assert running_info["name"] == source_id
        assert running_info["status"] == "Running"
        _assert_vm_row(
            service_client.get("/vms/list", timeout=30),
            source_id,
            status="Running",
            persistent=True,
        )

        exec_payload, timing = _measure_once(
            "service /vms/{id}/exec",
            lambda: service_client.post(
                f"/vms/{source_id}/exec",
                {
                    "command": "printf route-lifecycle-ok",
                    "timeout_secs": EXEC_TIMEOUT_SECS,
                },
                timeout=EXEC_TIMEOUT_SECS + 5,
            ),
            service_proc=service_proc,
        )
        assert exec_payload["exit_code"] == 0
        assert exec_payload["stdout"] == "route-lifecycle-ok"
        _assert_timing_budget(timing, p95_ms=10_000.0, max_ms=10_000.0, cpu_s=1.0)

        fork_payload, timing = _measure_once(
            "service /vms/{id}/fork",
            lambda: service_client.post(
                f"/vms/{source_id}/fork",
                {"name": child_id, "description": "Ironbank route lifecycle child"},
                timeout=60,
            ),
            service_proc=service_proc,
        )
        assert fork_payload["name"] == child_id
        assert fork_payload["size_bytes"] > 0
        _assert_timing_budget(timing, p95_ms=20_000.0, max_ms=20_000.0, cpu_s=5.0)
        child_status = service_client.get(f"/vms/{child_id}/status", timeout=30)
        assert child_status["id"] == child_id
        assert child_status["status"] == "Stopped"
        assert child_status["persistent"] is True
        assert child_status["can_resume"] is True

        delete_child, timing = _measure_once(
            "service /vms/{child}/delete",
            lambda: service_client.delete(f"/vms/{child_id}/delete", timeout=60),
            service_proc=service_proc,
        )
        assert delete_child == {"success": True}
        _assert_timing_budget(timing, p95_ms=5_000.0, max_ms=5_000.0, cpu_s=1.0)
        _assert_vm_absent(service_client.get("/vms/list", timeout=30), child_id)

        pause_payload, timing = _measure_once(
            "service /vms/{id}/pause",
            lambda: service_client.post(f"/vms/{source_id}/pause", {}, timeout=45),
            service_proc=service_proc,
        )
        assert pause_payload == {"success": True}
        _assert_timing_budget(timing, p95_ms=20_000.0, max_ms=20_000.0, cpu_s=5.0)
        suspended_status = service_client.get(f"/vms/{source_id}/status", timeout=30)
        assert suspended_status["status"] == "Suspended"
        assert suspended_status["persistent"] is True
        assert suspended_status["can_resume"] is True

        resume_payload, timing = _measure_once(
            "service /vms/{id}/resume from suspended",
            lambda: service_client.post(f"/vms/{source_id}/resume", {}, timeout=HTTP_TIMEOUT),
            service_proc=service_proc,
        )
        assert resume_payload["id"] == source_id
        assert resume_payload["profile_id"] == CODE_PROFILE_ID
        _assert_timing_budget(timing, p95_ms=45_000.0, max_ms=45_000.0, cpu_s=10.0)
        assert wait_exec_ready(service_client, source_id, timeout=EXEC_READY_TIMEOUT)

        stop_payload, timing = _measure_once(
            "service /vms/{id}/stop",
            lambda: service_client.post(f"/vms/{source_id}/stop", {}, timeout=30),
            service_proc=service_proc,
        )
        assert stop_payload == {"success": True, "persistent": True}
        _assert_timing_budget(timing, p95_ms=10_000.0, max_ms=10_000.0, cpu_s=2.0)
        stopped_status = service_client.get(f"/vms/{source_id}/status", timeout=30)
        assert stopped_status["status"] == "Stopped"
        assert stopped_status["persistent"] is True
        assert stopped_status["can_resume"] is True

        resume_payload, timing = _measure_once(
            "service /vms/{id}/resume from stopped",
            lambda: service_client.post(f"/vms/{source_id}/resume", {}, timeout=HTTP_TIMEOUT),
            service_proc=service_proc,
        )
        assert resume_payload["id"] == source_id
        _assert_timing_budget(timing, p95_ms=45_000.0, max_ms=45_000.0, cpu_s=10.0)
        assert wait_exec_ready(service_client, source_id, timeout=EXEC_READY_TIMEOUT)

        delete_source, timing = _measure_once(
            "service /vms/{id}/delete",
            lambda: service_client.delete(f"/vms/{source_id}/delete", timeout=60),
            service_proc=service_proc,
        )
        assert delete_source == {"success": True}
        _assert_timing_budget(timing, p95_ms=5_000.0, max_ms=5_000.0, cpu_s=1.0)
        _assert_vm_absent(service_client.get("/vms/list", timeout=30), source_id)

        purge_payload, timing = _measure_once(
            "service /purge",
            lambda: service_client.post("/purge", {"all": True}, timeout=60),
            service_proc=service_proc,
        )
        assert {"purged", "persistent_purged", "ephemeral_purged"} <= set(purge_payload)
        _assert_timing_budget(timing, p95_ms=5_000.0, max_ms=5_000.0, cpu_s=1.0)
    finally:
        if gateway is not None:
            gateway.stop()
        try:
            service.client().delete(f"/vms/{child_id}/delete", timeout=30)
        except Exception:
            pass
        try:
            service.client().delete(f"/vms/{source_id}/delete", timeout=30)
        except Exception:
            pass
        service.stop()
