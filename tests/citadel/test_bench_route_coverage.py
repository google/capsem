"""A timed GET does not cover a POST on the same path; cases retain their owner."""
from __future__ import annotations

from pathlib import Path

import pytest
import tomllib
from helpers.route_inventory import function_operations, route_operations

ROOT = Path(__file__).parents[2]
SERVICE = ROOT / "crates/capsem-service/src/router_runtime.rs"
GATEWAY = ROOT / "crates/capsem-gateway/src/main.rs"
RATIONALE = "[benchmark.routes] must name each HTTP method and successful fixture case; uncovered operations retain a concrete acceptance owner."


def config() -> dict:
    return tomllib.loads((ROOT / "config/gate.toml").read_text())["benchmark"]["routes"]


def measured_operations(coverage: dict) -> set[str]:
    cases = coverage["measured"]
    keys = [(case["method"], case["path"], case["case"]) for case in cases]
    assert len(keys) == len(set(keys)), RATIONALE
    assert all(case["case"].strip() and "*" not in case["path"] for case in cases), RATIONALE
    return {f'{case["method"]} {case["path"]}' for case in cases}


def validate(registered: set[str], coverage: dict) -> None:
    measured = measured_operations(coverage)
    debt, internal = set(coverage["unmeasured"]), set(coverage["internal"])
    assert not (measured & debt or measured & internal or debt & internal), RATIONALE
    assert measured | debt | internal == registered, (
        f"Missing: {sorted(registered - measured - debt - internal)}; "
        f"stale: {sorted((measured | debt | internal) - registered)}. {RATIONALE}"
    )
    keys = {f'{case["method"]} {case["path"]} [{case["case"]}]' for case in coverage["measured"]}
    for key, entry in coverage.get("unmeasured_cases", {}).items():
        operation, separator, name = key.rpartition(" [")
        assert "*" not in key and separator and name.endswith("]") and operation in registered and key not in keys, RATIONALE
        assert (ROOT / entry["owner"]).exists() and len(entry["reason"]) >= 20, RATIONALE
    for group in ("unmeasured", "internal"):
        for operation, entry in coverage[group].items():
            assert "*" not in operation and entry["owner"].strip() and len(entry["reason"].strip()) >= 20, RATIONALE
            if group == "unmeasured":
                assert (ROOT / entry["owner"]).exists(), RATIONALE


def registered_operations() -> set[str]:
    service = function_operations(SERVICE, "fn build_service_router")
    # Include gateway-owned registrations as well as the service proxy chain.
    # A hand-maintained health/events set would hide new gateway-only methods.
    extra = route_operations(GATEWAY.read_text())
    for filename in ("schema.rs", "surface.rs"):
        extra |= function_operations(GATEWAY.parent / filename, "fn routes()")
    return service | extra


def test_every_operation_has_exact_benchmark_ownership() -> None:
    validate(registered_operations(), config())


def test_gateway_method_topology_matches_service() -> None:
    service = function_operations(SERVICE, "fn build_service_router")
    proxied = function_operations(GATEWAY, "fn service_proxy_routes")
    own = {"POST /vms/{id}/exposures/{exposure_id}/preview-session"}
    assert proxied - service == own, RATIONALE
    assert service - proxied == {"GET /status", *config()["internal"]}, RATIONALE


def test_unmeasured_operations_remain_explicit_debt() -> None:
    assert len(config()["unmeasured"]) <= 70, RATIONALE
    assert len(config()["internal"]) == 7, RATIONALE


def test_parser_keeps_chained_methods_and_layered_handlers() -> None:
    body = 'Router::new().route("/one", get(read).post(write)).route("/two", post(resolve).layer(limit(12)))'
    assert route_operations(body) == {"GET /one", "POST /one", "POST /two"}


@pytest.mark.parametrize("body", ['.route(path, get(read))', '.route("/x", any(handler))', '.route("/x", get(read)).route("/x", get(other))'])
def test_parser_refuses_unclassified_or_duplicate_registration(body: str) -> None:
    with pytest.raises(AssertionError):
        route_operations(body)


def test_measured_get_does_not_cover_post_or_missing_case() -> None:
    coverage = {"measured": [{"method": "GET", "path": "/x", "case": "read"}], "unmeasured": {}, "internal": {}}
    with pytest.raises(AssertionError, match="POST /x"):
        validate({"GET /x", "POST /x"}, coverage)
    coverage["measured"].append(coverage["measured"][0])
    with pytest.raises(AssertionError):
        measured_operations(coverage)


def test_nested_handler_methods_do_not_invent_http_coverage() -> None:
    assert route_operations('.route("/x", get(make().post(payload)).layer(trace()))') == {"GET /x"}


def test_gateway_owned_method_additions_cannot_hide_behind_proxy_inventory(monkeypatch, tmp_path: Path) -> None:
    import sys

    source = GATEWAY.read_text().replace(
        '.route("/health", get(handle_health))',
        '.route("/health", get(handle_health).post(handle_health))',
    )
    gateway = tmp_path / "main.rs"
    gateway.write_text(source)
    for filename in ("schema.rs", "surface.rs"):
        (tmp_path / filename).write_text((GATEWAY.parent / filename).read_text())
    monkeypatch.setattr(sys.modules[__name__], "GATEWAY", gateway)
    with pytest.raises(AssertionError, match="POST /health"):
        validate(registered_operations(), config())
