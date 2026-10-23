"""A timed GET does not cover a POST on the same path; cases retain their owner."""
from __future__ import annotations

import tomllib
from pathlib import Path

import pytest
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
    streams = coverage.get("streaming", {})
    assert not (measured & set(streams)), RATIONALE
    measured |= set(streams)
    for operation, entry in streams.items():
        assert operation.startswith("GET ") and entry["collector"] == "stream", RATIONALE
        assert entry["target_scope"] in {"gateway", "both"} and len(entry["reason"]) >= 20, RATIONALE
        assert entry["cases"] and (ROOT / "benchmarks/collectors/stream").exists(), RATIONALE
        for case in entry["cases"].values():
            assert case["sample_key"] and case["metrics"], RATIONALE
    for domain in ("images", "policy"):
        finite = coverage.get(domain, {}).get("endpoints", {})
        assert not (measured & set(finite)), RATIONALE
        for entry in finite.values():
            assert entry["collector"] == "routes" and entry["target_scope"] == "both", RATIONALE
            assert entry["cases"] and len(entry["reason"]) >= 20, RATIONALE
        measured |= set(finite)
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
    assert len(config()["unmeasured"]) <= 44, RATIONALE
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


@pytest.mark.parametrize("operation,owner", [
    ("POST /proxies", "tests/ironbank/test_standalone_proxy.py"),
    ("POST /networks", "tests/ironbank/test_managed_network_lifecycle.py"),
    ("POST /restart", "tests/ironbank/test_sdk_restart.py"),
])
def test_mutation_debt_keeps_its_domain_fixture_owner(operation: str, owner: str) -> None:
    assert config()["unmeasured"][operation]["owner"] == owner, RATIONALE


def test_stream_evidence_cannot_overlap_polls_or_hide_missing_registered_method() -> None:
    import copy

    coverage = copy.deepcopy(config())
    operation = "GET /vms/{id}/stream"
    assert set(coverage["streaming"]) == {operation, "GET /events", "GET /vms/{id}/history/transcript"}, RATIONALE
    coverage["measured"].append({"method": "GET", "path": "/vms/{id}/stream", "case": "false-poll"})
    with pytest.raises(AssertionError):
        validate(registered_operations(), coverage)
    coverage = copy.deepcopy(config())
    del coverage["streaming"][operation]
    with pytest.raises(AssertionError, match="Missing"):
        validate(registered_operations(), coverage)


def test_finite_image_evidence_keeps_exact_operations_and_rejects_poll_overlap() -> None:
    import copy

    coverage = copy.deepcopy(config())
    assert set(coverage['images']['endpoints']) == {'GET /images', 'GET /images/{name}/inventory', 'POST /images/pull', 'POST /assets/ensure'}
    coverage['measured'].append({'method': 'POST', 'path': '/images/pull', 'case': 'false-fast-poll'})
    with pytest.raises(AssertionError):
        validate(registered_operations(), coverage)
    coverage = copy.deepcopy(config())
    del coverage['images']['endpoints']['POST /images/pull']
    with pytest.raises(AssertionError, match='Missing'):
        validate(registered_operations(), coverage)


def test_policy_mutation_cases_keep_host_scope_and_live_ledger_debt() -> None:
    import copy

    coverage = copy.deepcopy(config())
    operations = set(coverage['policy']['endpoints'])
    assert operations == {'PATCH /settings/edit', 'PUT /corp/edit', 'POST /corp/validate', 'PATCH /plugins/{plugin_id}/edit', 'PATCH /mcp/default/edit', 'PATCH /mcp/servers/{server_id}/tools/{tool_id}/edit'}, RATIONALE
    for operation, endpoint in coverage['policy']['endpoints'].items():
        assert len(endpoint['cases']) == 1 and set(endpoint['cases'].values()) == {'cycles'}, RATIONALE
        if operation.startswith('PATCH /mcp/') or '/plugins/' in operation:
            assert coverage['unmeasured_cases'][operation + ' [durable-ledger-read]']['owner'] == 'tests/ironbank/test_policy_mutation_routes.py', RATIONALE
            assert coverage['unmeasured_cases'][operation + ' [running-vm-publication]']['owner'] == 'crates/capsem-service/src/tests/policy_push.rs', RATIONALE
    coverage['measured'].append({'method': 'PATCH', 'path': '/settings/edit', 'case': 'fake-concurrent-write'})
    with pytest.raises(AssertionError):
        validate(registered_operations(), coverage)
    coverage = copy.deepcopy(config())
    del coverage['policy']['endpoints']['PUT /corp/edit']
    with pytest.raises(AssertionError, match='Missing'):
        validate(registered_operations(), coverage)
