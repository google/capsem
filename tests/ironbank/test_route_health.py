"""Ironbank route health gates for Capsem control surfaces.

These tests are intentionally black-box. They start the real service and gateway
processes, call the public routes the UI/TUI depend on, and fail when a route
quietly regresses into CPU-bound work such as hashing VM assets on a poll path.
"""

from __future__ import annotations

from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
import http.client
import json
import socket
import statistics
import threading
import time
import uuid
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
from tests.ironbank.test_stats_detail_contract import (
    SESSION_ID as SEEDED_SESSION_ID,
    _profile_contract,
    _seed_session_db,
    _write_registry,
)

SEEDED_VM_ID = "33333333-3333-4333-8333-333333333333"
CPU_ACCOUNTING_SLACK_S = 0.011

pytestmark = pytest.mark.integration


@dataclass(frozen=True)
class RouteContract:
    method: str
    path: str
    body: dict[str, Any] | None
    required_keys: set[str] | None
    response_kind: type
    forbidden_keys: set[str] | None = None


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
    def p99_ms(self) -> float:
        ordered = sorted(self.samples_ms)
        index = min(len(ordered) - 1, int(round((len(ordered) - 1) * 0.99)))
        return ordered[index]

    @property
    def max_ms(self) -> float:
        return max(self.samples_ms)


@dataclass(frozen=True)
class ConcurrentRouteWriteBenchmark:
    timing: RouteTiming
    writer_results: list[dict[str, Any]]
    final_default_action: str
    final_default_rule_id: str


class UnixHttpConnection(http.client.HTTPConnection):
    """Persistent HTTP/1.1 client over a Unix domain socket.

    The route-health benchmark must measure Capsem, not `curl` process startup.
    """

    def __init__(self, socket_path: str, *, timeout: float = 5.0):
        super().__init__("localhost", timeout=timeout)
        self.socket_path = socket_path

    def connect(self) -> None:
        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        sock.settimeout(self.timeout)
        sock.connect(self.socket_path)
        self.sock = sock


class TcpNoDelayHttpConnection(http.client.HTTPConnection):
    """Persistent HTTP/1.1 client with Nagle disabled for route timing."""

    def connect(self) -> None:
        super().connect()
        if self.sock is not None:
            self.sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)


class PersistentJsonClient:
    def __init__(
        self,
        connection_factory: Callable[[], http.client.HTTPConnection],
        *,
        auth_token: str | None = None,
    ):
        self._connection_factory = connection_factory
        self._conn = connection_factory()
        self._auth_token = auth_token

    def get(self, path: str, timeout: int = 20) -> Any:
        return self._request("GET", path, None, timeout=timeout)

    def post(self, path: str, body: dict[str, Any] | None = None, timeout: int = 20) -> Any:
        return self._request("POST", path, body, timeout=timeout)

    def close(self) -> None:
        self._conn.close()

    def _request(
        self,
        method: str,
        path: str,
        body: dict[str, Any] | None,
        *,
        timeout: int,
    ) -> Any:
        payload = None if body is None else json.dumps(body).encode("utf-8")
        headers = {"Content-Type": "application/json", "Connection": "keep-alive"}
        if self._auth_token is not None:
            headers["Authorization"] = f"Bearer {self._auth_token}"
        self._conn.timeout = timeout
        try:
            return self._request_once(method, path, payload, headers)
        except (http.client.RemoteDisconnected, BrokenPipeError, ConnectionResetError):
            self._conn.close()
            self._conn = self._connection_factory()
            return self._request_once(method, path, payload, headers)

    def _request_once(
        self,
        method: str,
        path: str,
        payload: bytes | None,
        headers: dict[str, str],
    ) -> Any:
        self._conn.request(method, path, body=payload, headers=headers)
        response = self._conn.getresponse()
        raw = response.read()
        assert 200 <= response.status < 300, (method, path, response.status, raw[:500])
        if not raw:
            return None
        return json.loads(raw.decode("utf-8"))


def _fast_service_client(service: ServiceInstance) -> PersistentJsonClient:
    return PersistentJsonClient(lambda: UnixHttpConnection(str(service.uds_path)))


def _fast_gateway_client(gateway: GatewayInstance) -> PersistentJsonClient:
    assert gateway.port is not None
    assert gateway.token is not None
    return PersistentJsonClient(
        lambda: TcpNoDelayHttpConnection("127.0.0.1", gateway.port, timeout=5.0),
        auth_token=gateway.token,
    )


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
    if contract.forbidden_keys is not None:
        assert contract.forbidden_keys.isdisjoint(payload), (contract.path, payload)


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
    samples: int = 64,
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


def _assert_timing_budget(
    timing: RouteTiming,
    *,
    p95_ms: float,
    max_ms: float | None,
    cpu_s: float,
    p99_ms: float | None = None,
) -> None:
    print(
        "ROUTE_HEALTH "
        f"{timing.label} p50={timing.p50_ms:.1f}ms "
        f"p95={timing.p95_ms:.1f}ms "
        f"p99={timing.p99_ms:.1f}ms max={timing.max_ms:.1f}ms "
        f"service_cpu={timing.service_cpu_s:.3f}s "
        f"gateway_cpu={timing.gateway_cpu_s if timing.gateway_cpu_s is not None else 'n/a'}"
    )
    assert timing.p95_ms <= p95_ms, (
        f"{timing.label} p95={timing.p95_ms:.1f}ms > {p95_ms}ms; samples={timing.samples_ms}"
    )
    if p99_ms is not None:
        assert timing.p99_ms <= p99_ms, (
            f"{timing.label} p99={timing.p99_ms:.1f}ms > {p99_ms}ms; samples={timing.samples_ms}"
        )
    if max_ms is not None:
        assert timing.max_ms <= max_ms, (
            f"{timing.label} max={timing.max_ms:.1f}ms > {max_ms}ms; samples={timing.samples_ms}"
        )
    # psutil reports process CPU from OS accounting ticks. On Linux that is
    # commonly 10ms, so tiny debug-build budgets need one tick of slack to
    # avoid failing on 0.10000000000000009 or a single scheduler tick.
    assert timing.service_cpu_s <= cpu_s + CPU_ACCOUNTING_SLACK_S, (
        f"{timing.label} service CPU={timing.service_cpu_s:.3f}s > {cpu_s:.3f}s"
    )
    if timing.gateway_cpu_s is not None:
        assert timing.gateway_cpu_s <= cpu_s + CPU_ACCOUNTING_SLACK_S, (
            f"{timing.label} gateway CPU={timing.gateway_cpu_s:.3f}s > {cpu_s:.3f}s"
        )


def route_timing_summary(timing: RouteTiming) -> dict[str, Any]:
    return {
        "label": timing.label,
        "samples": len(timing.samples_ms),
        "p50_ms": round(timing.p50_ms, 3),
        "p95_ms": round(timing.p95_ms, 3),
        "p99_ms": round(timing.p99_ms, 3),
        "max_ms": round(timing.max_ms, 3),
        "service_cpu_s": round(timing.service_cpu_s, 6),
        "gateway_cpu_s": (None if timing.gateway_cpu_s is None else round(timing.gateway_cpu_s, 6)),
    }


def _is_vm_scalar_state_route(path: str) -> bool:
    if "/vms/" not in path:
        return False
    suffix = path.split("/vms/", 1)[1].split("?", 1)[0]
    return suffix.count("/") == 1 and (suffix.endswith("/status") or suffix.endswith("/info"))


def _hot_route_budget(path: str, *, gateway: bool = False) -> tuple[float, float, float]:
    if path == "/status" and gateway:
        # Gateway status is not the service's scalar status route. It composes
        # service status, VM inventory, and profile/asset readiness into the
        # public dashboard payload. On Linux debug builds those service-owned
        # projections consume the same aggregate CPU envelope as gateway
        # /vms/list while remaining comfortably within the hot-route latency
        # budget. Keep the direct service /status budget at the scalar default.
        return (3.0, 8.0, 0.14)
    if _is_vm_scalar_state_route(path):
        # Per-session state routes touch richer lifecycle/profile metadata than
        # global scalar status, but must still stay memory-backed and responsive
        # enough for TUI/UI polling. Linux debug builds route a running VM
        # through session DB readiness/status and storage DTO assembly; keep
        # the aggregate CPU budget above the measured release-gate baseline
        # without allowing projection rebuilds or route-time SQLite ownership.
        return (
            3.0 if not gateway else 4.0,
            8.0 if not gateway else 10.0,
            0.15 if not gateway else 0.19,
        )
    if path == "/vms/list":
        # Empty-list polling is cheaper, but an active VM row includes lifecycle
        # and profile metadata. Keep the budget memory-backed while allowing the
        # one-VM debug-build route-health loop observed on Linux.
        return (
            3.0 if not gateway else 4.0,
            8.0 if not gateway else 10.0,
            0.10 if not gateway else 0.14,
        )
    if path == "/profiles/status":
        # Profile status is a richer readiness payload than scalar service
        # status: it returns per-profile asset readiness, manifest provenance,
        # and launchability fields. It must stay cache backed, but Linux debug
        # builds can account an extra service CPU tick when measured through the
        # gateway proxy loop.
        return (
            3.0 if not gateway else 4.0,
            8.0 if not gateway else 10.0,
            0.08 if not gateway else 0.12,
        )
    if "/stats/detail" in path:
        # Detail is the largest user-facing ledger route and includes body blob
        # metadata/content. It must still be DB-handle/memory backed rather
        # than rebuilding projections or blocking on route-owned SQLite. The
        # response body is large enough that debug-build JSON encoding can burn
        # more aggregate CPU than scalar hot routes across 64 samples.
        return (
            12.0 if not gateway else 16.0,
            35.0 if not gateway else 45.0,
            0.70 if not gateway else 0.90,
        )
    if any(
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
        # Ledger list/status routes should be near-constant DB-handle reads.
        # Aggregate security status routes cross enough rows that debug JSON
        # encoding can brush into the 6ms range locally; CPU remains the
        # regression tripwire for route-time disk scans/projection rebuilds.
        # Gateway measurements include proxy hop jitter, but must stay
        # sub-10ms p95.
        aggregate_security = any(
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
        return (
            7.0 if not gateway else 9.0,
            14.0 if not gateway else 18.0,
            (0.30 if not gateway else 0.42)
            if aggregate_security
            else (0.14 if not gateway else 0.20),
        )
    if path.endswith("/assets/status"):
        # Asset status returns a richer per-file readiness payload than scalar
        # status routes. It is still byte-cache backed; this budget keeps 64
        # debug-build calls bounded and catches any return to file hashing or
        # route-time DB reads.
        return (
            3.0 if not gateway else 4.0,
            8.0 if not gateway else 10.0,
            0.16 if not gateway else 0.22,
        )
    if path.endswith("/mcp/default/info"):
        # Default MCP info returns the profile's full builtin/default server
        # shape. Gateway JSON buffering adds a proxy/body materialization hop;
        # on Linux debug builds the full route loop can account ~0.17s service
        # CPU across 64 samples while remaining a small in-memory response.
        return (
            2.0 if not gateway else 5.0,
            5.0 if not gateway else 10.0,
            0.10 if not gateway else 0.19,
        )
    if path.endswith("/mcp/servers/list"):
        # MCP server inventory is still cache backed, but larger than scalar
        # profile info and measured through the gateway proxy loop it can use
        # more service CPU ticks than the default tiny-route budget.
        return (
            3.0 if not gateway else 4.0,
            6.0 if not gateway else 9.0,
            0.06 if not gateway else 0.14,
        )
    if path.endswith("/rules/list"):
        # Rule-inventory routes return the in-memory compiled/default rule
        # shape. They are larger than scalar info/status routes and must stay
        # comfortably sub-2ms without reparsing rule files or touching SQLite.
        return (
            2.0 if not gateway else 3.0,
            5.0 if not gateway else 8.0,
            0.08 if not gateway else 0.12,
        )
    if path.endswith("/latest"):
        # Latest ledgers are projection backed. They can be a larger array than
        # status routes, but route-time SQLite reads would show up immediately
        # in this loop.
        return (
            3.0 if not gateway else 4.0,
            8.0 if not gateway else 10.0,
            0.10 if not gateway else 0.14,
        )
    if path.endswith("/evaluate"):
        # Rule evaluation is not a passive poll route, but it is the enforcement
        # control plane and must stay memory/compiler-cache backed. This catches
        # accidental route-time SQLite or rule-file reloads on every decision.
        # Full-suite debug builds can account just under 0.09s service CPU for
        # 64 cached decisions while still staying sub-2ms p95.
        return (
            2.0 if not gateway else 3.0,
            5.0 if not gateway else 8.0,
            0.10 if not gateway else 0.14,
        )
    if "/plugins/" in path and (path.endswith("/info") or path.endswith("/credentials/info")):
        # Plugin and credential inventory routes hydrate runtime counters from
        # in-memory projections. Keep them off Keychain/SQLite/hashing paths.
        return (
            3.0 if not gateway else 4.0,
            8.0 if not gateway else 10.0,
            0.10 if not gateway else 0.14,
        )
    if path == "/stats":
        # `/stats` is byte-projection backed, but it is still one of the
        # larger hot JSON bodies. The gateway adds a TCP hop, one UDS proxy
        # connection, and JSON response buffering per request, so the gateway
        # p95 budget tracks the Linux release-gate baseline while staying well
        # under the broader hot-route envelope.
        return (
            2.0 if not gateway else 4.0,
            5.0 if not gateway else 9.0,
            0.12 if gateway else 0.08,
        )
    return (
        2.0 if not gateway else 3.0,
        5.0 if not gateway else 8.0,
        0.05 if not gateway else 0.08,
    )


def _hot_route_contracts(profile: str) -> list[RouteContract]:
    return [
        RouteContract("GET", "/status", None, {"ready", "service"}, dict),
        RouteContract("GET", "/vms/list", None, {"sandboxes"}, dict),
        RouteContract("GET", "/stats", None, {"global", "sessions"}, dict),
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
            f"/profiles/{profile}/assets/status",
            None,
            {"profile_id", "ready", "assets"},
            dict,
        ),
        RouteContract(
            "GET",
            f"/profiles/{profile}/plugins/list",
            None,
            {"scope", "plugins"},
            dict,
        ),
        RouteContract(
            "GET",
            f"/profiles/{profile}/plugins/info",
            None,
            {"scope", "plugin_count", "enabled_count"},
            dict,
        ),
        RouteContract(
            "GET",
            f"/profiles/{profile}/plugins/credential_broker/info",
            None,
            {"id", "name", "description", "stage", "config", "runtime"},
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
        RouteContract("GET", "/security/status", None, {"sessions", "total"}, dict),
        RouteContract("GET", "/security/latest", None, None, list),
        RouteContract("GET", "/enforcement/status", None, {"sessions", "total"}, dict),
        RouteContract("GET", "/enforcement/latest", None, None, list),
        RouteContract("GET", "/detection/status", None, {"sessions", "total"}, dict),
        RouteContract("GET", "/detection/latest", None, None, list),
    ]


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


def _assert_uuid_route_id(vm_id: str) -> None:
    parsed = uuid.UUID(vm_id)
    assert str(parsed) == vm_id


def _service_route_contracts() -> list[RouteContract]:
    profile = CODE_PROFILE_ID
    return [
        RouteContract("GET", "/status", None, {"components", "ready", "service", "version"}, dict),
        RouteContract("GET", "/version", None, {"version"}, dict),
        RouteContract("GET", "/vms/list", None, {"sandboxes"}, dict),
        RouteContract(
            "POST", "/purge", {}, {"purged", "persistent_purged", "ephemeral_purged"}, dict
        ),
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


def _seeded_session_route_contracts(session_id: str) -> list[RouteContract]:
    return [
        RouteContract("GET", f"/vms/{session_id}/status", None, {"id", "name", "status"}, dict),
        RouteContract("GET", f"/vms/{session_id}/info", None, {"id", "name", "status"}, dict),
        RouteContract(
            "GET",
            f"/vms/{session_id}/stats/detail",
            None,
            {
                "model_stats",
                "model_events",
                "tool_events",
                "http_events",
                "dns_events",
                "file_events",
                "process_events",
                "audit_events",
                "credential_events",
                "body_blobs",
            },
            dict,
        ),
        RouteContract(
            "GET",
            f"/vms/{session_id}/history?layer=net&limit=50",
            None,
            {"commands", "total"},
            dict,
        ),
        RouteContract(
            "GET",
            f"/vms/{session_id}/history/counts",
            None,
            {"exec_count", "audit_count"},
            dict,
        ),
        RouteContract(
            "GET",
            f"/vms/{session_id}/history/processes",
            None,
            {"processes"},
            dict,
        ),
        RouteContract(
            "GET",
            f"/vms/{session_id}/timeline?layers=net,model,tools,dns,fs,exec,security&limit=100",
            None,
            {"columns", "rows"},
            dict,
        ),
        RouteContract(
            "GET",
            f"/vms/{session_id}/security/status",
            None,
            {"total", "by_action", "by_level", "by_event_type"},
            dict,
        ),
        RouteContract(
            "GET",
            f"/vms/{session_id}/security/latest?limit=50",
            None,
            None,
            list,
        ),
        RouteContract(
            "GET",
            f"/vms/{session_id}/detection/status",
            None,
            {"total", "by_action", "by_level", "by_event_type"},
            dict,
        ),
        RouteContract(
            "GET",
            f"/vms/{session_id}/detection/latest?limit=50",
            None,
            None,
            list,
        ),
        RouteContract(
            "GET",
            f"/vms/{session_id}/enforcement/status",
            None,
            {"total", "by_action", "by_level", "by_event_type"},
            dict,
        ),
        RouteContract(
            "GET",
            f"/vms/{session_id}/enforcement/latest?limit=50",
            None,
            None,
            list,
        ),
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


def test_vm_list_table_record_uses_uuid_id_not_display_name_blackbox() -> None:
    service = ServiceInstance()
    client = None
    requested_name = vm_name("ironbank-id-row")
    vm_id: str | None = None
    try:
        service.start()
        client = service.client()
        created = client.post(
            "/vms/create",
            {
                "name": requested_name,
                "profile_id": CODE_PROFILE_ID,
                "ram_mb": DEFAULT_RAM_MB,
                "cpus": DEFAULT_CPUS,
                "persistent": True,
            },
            timeout=90,
        )
        assert created is not None
        vm_id = created["id"]
        assert created["name"] == requested_name
        assert vm_id != requested_name
        _assert_uuid_route_id(vm_id)

        listing = client.get("/vms/list", timeout=30)
        row = _assert_vm_row(listing, vm_id, status="Running", persistent=True)
        assert row["id"] == vm_id
        assert row["name"] == requested_name
        assert row["id"] != row["name"]

        info = client.get(f"/vms/{vm_id}/info", timeout=30)
        assert info["id"] == vm_id
        assert info["name"] == requested_name
    finally:
        if client is not None:
            try:
                client.delete(f"/vms/{vm_id or requested_name}/delete", timeout=60)
            except Exception:
                pass
        service.stop()


@pytest.mark.serial
def test_hot_control_routes_have_latency_and_cpu_budgets() -> None:
    service = ServiceInstance()
    gateway: GatewayInstance | None = None
    fast_service_client: PersistentJsonClient | None = None
    fast_gateway_client: PersistentJsonClient | None = None
    try:
        service.start()
        gateway = GatewayInstance(uds_path=service.uds_path)
        gateway.start()
        fast_service_client = _fast_service_client(service)
        fast_gateway_client = _fast_gateway_client(gateway)
        assert service.proc is not None
        assert gateway.proc is not None
        service_proc = psutil.Process(service.proc.pid)
        gateway_proc = psutil.Process(gateway.proc.pid)

        for contract in _hot_route_contracts(CODE_PROFILE_ID):
            timing = _measure_route(
                f"service {contract.path}",
                lambda c=contract: _assert_contract(fast_service_client, c),
                service_proc=service_proc,
            )
            p95_ms, max_ms, cpu_s = _hot_route_budget(contract.path)
            _assert_timing_budget(timing, p95_ms=p95_ms, max_ms=max_ms, cpu_s=cpu_s)

        hot_gateway_routes = [
            RouteContract(
                "GET",
                "/status",
                None,
                {"gateway_version", "service", "vm_count", "profiles"},
                dict,
                {"assets"},
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
            RouteContract("GET", "/stats", None, {"global", "sessions"}, dict),
            *_hot_route_contracts(CODE_PROFILE_ID)[4:],
        ]
        for contract in hot_gateway_routes:
            timing = _measure_route(
                f"gateway {contract.path}",
                lambda c=contract: _assert_contract(fast_gateway_client, c),
                service_proc=service_proc,
                gateway_proc=gateway_proc,
            )
            p95_ms, max_ms, cpu_s = _hot_route_budget(contract.path, gateway=True)
            _assert_timing_budget(timing, p95_ms=p95_ms, max_ms=max_ms, cpu_s=cpu_s)
    finally:
        if fast_service_client is not None:
            fast_service_client.close()
        if fast_gateway_client is not None:
            fast_gateway_client.close()
        if gateway is not None:
            gateway.stop()
        service.stop()


@pytest.mark.serial
def test_seeded_session_ledger_routes_have_latency_and_cpu_budgets() -> None:
    service = ServiceInstance()
    gateway: GatewayInstance | None = None
    fast_service_client: PersistentJsonClient | None = None
    fast_gateway_client: PersistentJsonClient | None = None
    try:
        session_dir = service.tmp_dir / "persistent" / SEEDED_VM_ID
        session_dir.mkdir(parents=True, exist_ok=True)
        contract = _profile_contract(service.tmp_dir)
        _seed_session_db(session_dir / "session.db")
        _write_registry(service.tmp_dir, session_dir, contract)
        registry_path = service.tmp_dir / "persistent_registry.json"
        registry = json.loads(registry_path.read_text())
        registry["vms"][SEEDED_SESSION_ID]["id"] = SEEDED_VM_ID
        registry_path.write_text(json.dumps(registry))

        service.start()
        gateway = GatewayInstance(uds_path=service.uds_path)
        gateway.start()
        fast_service_client = _fast_service_client(service)
        fast_gateway_client = _fast_gateway_client(gateway)
        assert service.proc is not None
        assert gateway.proc is not None
        service_proc = psutil.Process(service.proc.pid)
        gateway_proc = psutil.Process(gateway.proc.pid)

        for client_label, client, gateway_for_cpu in (
            ("service", fast_service_client, None),
            ("gateway", fast_gateway_client, gateway_proc),
        ):
            is_gateway = gateway_for_cpu is not None
            for route_contract in _seeded_session_route_contracts(SEEDED_VM_ID):
                timing = _measure_route(
                    f"{client_label} {route_contract.path}",
                    lambda c=route_contract, route_client=client: _assert_contract(
                        route_client,
                        c,
                    ),
                    service_proc=service_proc,
                    gateway_proc=gateway_for_cpu,
                )
                summary = route_timing_summary(timing)
                print("ROUTE_LATENCY_JSON " + json.dumps(summary, sort_keys=True))
                p95_ms, max_ms, cpu_s = _hot_route_budget(
                    route_contract.path,
                    gateway=is_gateway,
                )
                _assert_timing_budget(
                    timing,
                    p95_ms=p95_ms,
                    max_ms=max_ms,
                    cpu_s=cpu_s,
                )
    finally:
        if fast_service_client is not None:
            fast_service_client.close()
        if fast_gateway_client is not None:
            fast_gateway_client.close()
        if gateway is not None:
            gateway.stop()
        service.stop()


def run_concurrent_route_read_write_benchmark(
    *,
    samples: int = 96,
    mutation_repeats: int = 4,
) -> ConcurrentRouteWriteBenchmark:
    service = ServiceInstance()
    fast_service_client: PersistentJsonClient | None = None
    try:
        service.start()
        fast_service_client = _fast_service_client(service)
        writer_client = service.client()
        assert service.proc is not None
        service_proc = psutil.Process(service.proc.pid)
        writer_started = threading.Event()
        writer_done = threading.Event()
        writer_results: list[dict[str, Any]] = []
        writer_errors: list[BaseException] = []

        def write_profile_mutations() -> None:
            try:
                writer_started.set()
                actions = ("allow", "ask", "block") * mutation_repeats
                for index, action in enumerate(actions):
                    response = writer_client.patch(
                        f"/profiles/{CODE_PROFILE_ID}/mcp/default/edit",
                        {"action": action},
                        timeout=30,
                    )
                    assert response["profile_id"] == CODE_PROFILE_ID
                    assert response["action"] == action
                    assert response["mutation"]["target_kind"] == "mcp_default"
                    assert response["mutation"]["operation"] == "permission"
                    assert response["mutation"]["mutation_id"]
                    writer_results.append(response)
                    # Keep the writer active long enough for read/write
                    # overlap without inventing a fake DB path.
                    if index < len(actions) - 1:
                        time.sleep(0.002)
            except BaseException as error:  # pragma: no cover - surfaced below
                writer_errors.append(error)
            finally:
                writer_done.set()

        with ThreadPoolExecutor(max_workers=1) as executor:
            writer = executor.submit(write_profile_mutations)
            assert writer_started.wait(timeout=5), "writer route never started"

            timing = _measure_route(
                "service /stats during profile-mutation writes",
                lambda: _assert_contract(
                    fast_service_client,
                    RouteContract("GET", "/stats", None, {"global", "sessions"}, dict),
                ),
                service_proc=service_proc,
                samples=samples,
            )

            writer.result(timeout=30)

        assert not writer_errors, writer_errors
        assert writer_done.is_set()

        final_default = fast_service_client.get(
            f"/profiles/{CODE_PROFILE_ID}/mcp/default/info",
            timeout=20,
        )
        return ConcurrentRouteWriteBenchmark(
            timing=timing,
            writer_results=writer_results,
            final_default_action=final_default["action"],
            final_default_rule_id=final_default["rule_id"],
        )
    finally:
        if fast_service_client is not None:
            fast_service_client.close()
        service.stop()


@pytest.mark.serial
def test_concurrent_route_reads_while_writes_are_active() -> None:
    """Route reads must stay responsive while public routes write ledgers.

    This is the S05 disk-backed baseline before DB-owned memory tables. The
    reader path is `/stats`, which reads through the main DB handle. The writer
    path is a public profile mutation route, which writes
    `profile_mutation_events` through the same DB boundary. No direct SQLite
    fixture writes are allowed here: the point is to measure the user-visible
    route contract while Capsem is doing real service work.
    """

    # Keep IronBank identical to the archived route-latency benchmark: one
    # black-box workload and one release-blocking budget, not two drifting
    # calibrations for the same public routes.
    result = run_concurrent_route_read_write_benchmark(samples=160, mutation_repeats=8)
    assert len(result.writer_results) == 24
    assert {row["action"] for row in result.writer_results} == {"allow", "ask", "block"}
    # This overlaps 160 `/stats` reads with 24 real profile mutation writes. Gate
    # the tail on p99, matching the route-latency benchmark contract, so one
    # scheduler outlier does not fail a run whose p95 and CPU prove the route
    # stayed projection-backed.
    _assert_timing_budget(
        result.timing,
        p95_ms=15.0,
        p99_ms=40.0,
        max_ms=None,
        cpu_s=0.34,
    )

    assert result.final_default_action == result.writer_results[-1]["action"]
    assert result.final_default_rule_id


@pytest.mark.serial
def test_vm_session_lifecycle_routes_have_state_and_latency_budgets() -> None:
    service = ServiceInstance()
    gateway: GatewayInstance | None = None
    source_name = vm_name("ironbank-route-life")
    child_name = vm_name("ironbank-route-child")
    source_id: str | None = None
    child_id: str | None = None
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
                    "name": source_name,
                    "profile_id": CODE_PROFILE_ID,
                    "ram_mb": DEFAULT_RAM_MB,
                    "cpus": DEFAULT_CPUS,
                    "persistent": True,
                },
                timeout=HTTP_TIMEOUT,
            ),
            service_proc=service_proc,
        )
        source_id = create["id"]
        assert create["name"] == source_name
        assert source_id != source_name
        _assert_uuid_route_id(source_id)
        assert create["profile_id"] == CODE_PROFILE_ID
        _assert_timing_budget(timing, p95_ms=45_000.0, max_ms=45_000.0, cpu_s=10.0)
        assert wait_exec_ready(service_client, source_id, timeout=EXEC_READY_TIMEOUT)

        for client_label, client, gateway_for_cpu in (
            ("service", service_client, None),
            ("gateway", gateway_client, gateway_proc),
        ):
            for contract in (
                RouteContract(
                    "GET", f"/vms/{source_id}/status", None, {"id", "name", "status"}, dict
                ),
                RouteContract(
                    "GET", f"/vms/{source_id}/info", None, {"id", "name", "status"}, dict
                ),
                RouteContract(
                    "GET",
                    f"/vms/{source_id}/stats/detail",
                    None,
                    {
                        "model_stats",
                        "model_events",
                        "tool_events",
                        "http_events",
                        "dns_events",
                        "file_events",
                        "process_events",
                        "audit_events",
                        "credential_events",
                        "body_blobs",
                    },
                    dict,
                ),
                RouteContract("GET", "/vms/list", None, {"sandboxes"}, dict),
            ):
                timing = _measure_route(
                    f"{client_label} {contract.path}",
                    lambda c=contract, route_client=client: _assert_contract(route_client, c),
                    service_proc=service_proc,
                    gateway_proc=gateway_for_cpu,
                )
                p95_ms, max_ms, cpu_s = _hot_route_budget(
                    contract.path,
                    gateway=gateway_for_cpu is not None,
                )
                _assert_timing_budget(
                    timing,
                    p95_ms=max(p95_ms, 350.0),
                    max_ms=max(max_ms, 500.0),
                    cpu_s=cpu_s,
                )

        running_status = service_client.get(f"/vms/{source_id}/status", timeout=30)
        assert running_status["id"] == source_id
        assert running_status["name"] == source_name
        assert running_status["status"] == "Running"
        assert running_status["persistent"] is True
        assert running_status["can_resume"] is False
        assert running_status["available_actions"] == ["pause", "stop", "fork", "delete"]
        running_info = service_client.get(f"/vms/{source_id}/info", timeout=30)
        assert running_info["profile_id"] == CODE_PROFILE_ID
        assert running_info["name"] == source_name
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
                {"name": child_name, "description": "Ironbank route lifecycle child"},
                timeout=60,
            ),
            service_proc=service_proc,
        )
        child_id = fork_payload["id"]
        assert fork_payload["name"] == child_name
        assert child_id != child_name
        _assert_uuid_route_id(child_id)
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
