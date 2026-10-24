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
    for domain in ("images", "policy", "lifecycle", "networks", "proxies", "workload_controls", "network_members", "credentials", "mcp_controls", "run_purge", "save_controls", "ask_controls"):
        finite = coverage.get(domain, {}).get("endpoints", {})
        assert not (measured & set(finite)), RATIONALE
        for entry in finite.values():
            assert entry["collector"] == "routes" and entry["target_scope"] == "both", RATIONALE
            assert entry["cases"] and len(entry["reason"]) >= 20, RATIONALE
        measured |= set(finite)
    preview = coverage.get('preview_controls',{}).get('endpoints',{})
    assert not measured & set(preview), RATIONALE
    gateway_mints = {'POST /vms/{id}/surface/session','POST /vms/{id}/exposures/{exposure_id}/preview-session'}
    for operation,entry in preview.items():
        assert entry['collector'] == 'routes' and entry['target_scope'] == ('gateway' if operation in gateway_mints else 'both'), RATIONALE
        assert entry['cases'] and len(entry['reason']) >= 20, RATIONALE
    measured |= set(preview)
    debt, internal = set(coverage["unmeasured"]), set(coverage["internal"])
    errors = coverage.get('connection_errors',{})
    if errors:
        assert errors['success_coverage'] is False, RATIONALE
        error_ops = set(errors['endpoints'])
        assert error_ops <= debt and not error_ops & (measured | internal), RATIONALE
        assert all(entry['status'] in {404,503} and entry['classification'] and entry['code'] for entry in errors['endpoints'].values()), RATIONALE
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
    assert len(config()["unmeasured"]) <= 14, RATIONALE
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


def test_lifecycle_bindings_are_six_finite_mutations_and_keep_debt():
    import copy

    coverage = copy.deepcopy(config())
    expected = {'POST /vms/create', 'POST /vms/{id}/exec', 'POST /vms/{id}/fork', 'POST /vms/{id}/start', 'POST /vms/{id}/stop', 'DELETE /vms/{id}/delete'}
    assert set(coverage['lifecycle']['endpoints']) == expected, RATIONALE
    assert 'POST /vms/{id}/resume' in coverage['unmeasured'], RATIONALE
    assert set(coverage['save_controls']['endpoints']) == {'POST /vms/{id}/save'}, RATIONALE
    assert set(coverage['run_purge']['endpoints']) == {'POST /run','POST /purge'}, RATIONALE
    assert 'POST /vms/{id}/stop [physical-producer-ledger-drain]' in coverage['unmeasured_cases'], RATIONALE
    assert 'POST /vms/{id}/exec [per-request-cpu-limits]' in coverage['unmeasured_cases'], RATIONALE
    del coverage['lifecycle']['endpoints']['POST /vms/create']
    with pytest.raises(AssertionError, match='Missing'):
        validate(registered_operations(), coverage)
    coverage = copy.deepcopy(config())
    coverage['measured'].append({'method': 'POST', 'path': '/vms/create', 'case': 'fake-concurrent-create'})
    with pytest.raises(AssertionError):
        validate(registered_operations(), coverage)


def test_finite_empty_network_and_synthetic_diagnostics_bind_exact_four_operations():
    import copy
    coverage = copy.deepcopy(config())
    assert set(coverage['networks']['endpoints']) == {'POST /networks', 'DELETE /networks/{id}'}, RATIONALE
    diagnostics = [case for case in coverage['measured'] if case.get('response') == 'diagnostics']
    assert len(diagnostics) == 6 and {case['path'] for case in diagnostics} == {'/panics', '/triage'}, RATIONALE
    assert {case['case'] for case in diagnostics} == {'synthetic-populated', 'synthetic-limited', 'synthetic-later-since'}, RATIONALE
    assert set(coverage['network_members']['endpoints']) == {'GET /networks/{id}/logs', 'PUT /networks/{id}/members/{vm_id}', 'DELETE /networks/{id}/members/{vm_id}'}, RATIONALE
    operations = measured_operations(coverage) | set(coverage['streaming'])
    for domain in ('images', 'policy', 'lifecycle', 'networks', 'proxies', 'workload_controls', 'network_members', 'credentials', 'mcp_controls', 'preview_controls', 'run_purge', 'save_controls', 'ask_controls'):
        operations |= set(coverage[domain]['endpoints'])
    assert len(operations) == 103 and len(coverage['unmeasured']) == 13, RATIONALE
    assert len(coverage['internal']) == 7 and len(coverage['unmeasured_cases']) == 19, RATIONALE
    del coverage['networks']['endpoints']['POST /networks']
    with pytest.raises(AssertionError, match='POST /networks'):
        validate(registered_operations(), coverage)


def test_proxy_original_operations_cannot_overlap_polling_or_disappear():
    import copy
    coverage = copy.deepcopy(config())
    endpoints = coverage['proxies']['endpoints']
    assert set(endpoints) == {'POST /proxies', 'POST /proxies/{id}/heartbeat', 'POST /proxies/{id}/stop'}, RATIONALE
    del endpoints['POST /proxies/{id}/heartbeat']
    with pytest.raises(AssertionError, match=r'POST /proxies/\{id\}/heartbeat'):
        validate(registered_operations(), coverage)


def test_genuine_browser_upload_cases_keep_physical_archive_proof_explicit():
    import copy
    coverage = copy.deepcopy(config())
    cases = coverage['workload_controls']['endpoints']
    assert set(cases) == {'POST /vms/{id}/browser/acknowledge','POST /vms/{id}/files/content'}, RATIONALE
    assert set(cases['POST /vms/{id}/files/content']['cases']) == {'bytes-4096','bytes-1048576'}, RATIONALE
    assert coverage['unmeasured_cases']['POST /vms/{id}/files/content [physical-import-archive]']['owner'] == 'tests/ironbank/test_files_process_ledger.py', RATIONALE
    del cases['POST /vms/{id}/files/content']
    with pytest.raises(AssertionError,match='files/content'):
        validate(registered_operations(),coverage)


def test_disabled_error_paths_cannot_launder_success_or_auth_failures():
    import copy
    coverage = copy.deepcopy(config())
    errors = coverage['connection_errors']
    assert len(errors['endpoints']) == 7 and errors['success_coverage'] is False, RATIONALE
    operation = 'POST /connections/google/authorize'
    del coverage['unmeasured'][operation]
    with pytest.raises(AssertionError):
        validate(registered_operations(),coverage)
    coverage = copy.deepcopy(config())
    coverage['connection_errors']['success_coverage'] = True
    with pytest.raises(AssertionError):
        validate(registered_operations(),coverage)


def test_run_purge_save_are_finite_original_mutations_not_missing_or_read_polls():
    import copy
    for operation in ('POST /run','POST /purge','POST /vms/{id}/save'):
        coverage = copy.deepcopy(config())
        domain = 'save_controls' if operation.endswith('/save') else 'run_purge'
        del coverage[domain]['endpoints'][operation]
        with pytest.raises(AssertionError,match='Missing'):
            validate(registered_operations(),coverage)
        coverage = copy.deepcopy(config())
        coverage['measured'].append({'method':'POST','path':operation.split(' ',1)[1],'case':'fake-concurrent-mutation'})
        with pytest.raises(AssertionError):
            validate(registered_operations(),coverage)


def test_original_ask_decision_cannot_disappear_or_inherit_a_read_poll():
    import copy
    operation = 'POST /vms/{id}/security/asks/{ask_id}/resolve'
    coverage = copy.deepcopy(config())
    assert coverage['ask_controls']['endpoints'][operation]['cases'] == {'approve-original':'cycles','deny-original':'cycles'}, RATIONALE
    del coverage['ask_controls']['endpoints'][operation]
    with pytest.raises(AssertionError,match='Missing'):
        validate(registered_operations(),coverage)
