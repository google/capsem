"""Successful route cases keep method, fixture identity and measurements distinct."""
from __future__ import annotations

import pytest
from helpers.stopped_workspace import VM_ID

from benchmarks.helpers.route_cases import RouteCase, measured_cases
from benchmarks.helpers.route_validation import validate_case, validate_snapshot


def test_slice_one_and_two_are_successful_cases_not_method_wildcards() -> None:
    cases = measured_cases()
    assert len(cases) == 49
    assert len({case.metric_key for case in cases}) == len(cases)
    assert len({case.path for case in cases}) == 46
    assert {case.case for case in cases if "{plugin_id}" in case.path} == {
        "credential_broker", "pii_filter", "secret_exfiltration", "utf8_guard",
    }
    for case in cases:
        assert "{" not in case.resolve("live-id")
        if case.live:
            with pytest.raises(ValueError, match="live"):
                case.resolve(None)
    assert any(case.path.endswith("/security/asks") and case.case == "idle" for case in cases)


def test_plugin_cases_resolve_to_distinct_real_ids() -> None:
    case = RouteCase("GET", "/plugins/{plugin_id}/info", "pii_filter")
    assert case.resolve(None) == "/plugins/pii_filter/info"
    assert case.metric_key != RouteCase("GET", case.path, "utf8_guard").metric_key
    with pytest.raises(ValueError, match="fixture"):
        RouteCase("GET", "/plugins/{plugin_id}/info", "invented").resolve(None)


@pytest.mark.parametrize("path,payload", [
    ("/vms/{id}/stats/summary", {"total_requests": 0}),
    ("/vms/{id}/info", {"id": VM_ID, "persistent": True, "session_db": {"ready": False}}),
    ("/vms/{id}/timeline", {"events": []}),
    ("/vms/{id}/history/counts", {"exec_count": True, "audit_count": 0}),
    ("/plugins/{plugin_id}/info", {"id": "wrong"}),
    ("/settings/info", {"tree": [], "issues": []}),
    ("/vms/{id}/container", {"state": "failed", "image": "test", "error": "broken"}),
    ("/vms/{id}/security/asks", {"vm_id": "live-id", "generation": "boot", "pending_count": 1, "requests": []}),
])
def test_fast_empty_wrong_or_failed_responses_do_not_qualify(path: str, payload: dict) -> None:
    case = RouteCase("GET", path, "pii_filter" if "{plugin_id}" in path else "idle")
    with pytest.raises((AssertionError, ValueError)):
        validate_case(case, payload, "live-id")


def test_security_counters_reconcile_instead_of_accepting_arbitrary_json() -> None:
    case = RouteCase("GET", "/vms/{id}/enforcement/status", "recorded-session")
    validate_case(case, {"total": 0, "by_action": [], "by_event_type": [], "by_level": [], "by_rule": []}, None)
    with pytest.raises(AssertionError):
        validate_case(case, {"total": 1, "by_action": [], "by_event_type": [], "by_level": [], "by_rule": []}, None)


def test_public_snapshot_counters_must_agree_and_have_recorded_traffic() -> None:
    info = {"total_requests": 2, "total_input_tokens": 3, "total_output_tokens": 4}
    summary = dict(info)
    validate_snapshot({"/vms/{id}/info": info, "/vms/{id}/stats/summary": summary})
    summary["total_requests"] = 0
    with pytest.raises(AssertionError):
        validate_snapshot({"/vms/{id}/info": info, "/vms/{id}/stats/summary": summary})


def test_canonical_sdk_schema_rejects_wrong_wire_types_before_timing() -> None:
    from benchmarks.helpers.route_validation import validate_wire

    case = RouteCase("GET", "/mcp/info", "local-config")
    validate_wire(case, b'{"server_count":1,"manual_server_count":0,"builtin_local_enabled":true}', None)
    with pytest.raises(ValueError):
        validate_wire(case, b'{"server_count":"1","manual_server_count":0,"builtin_local_enabled":true}', None)


def test_current_plugin_dto_mode_and_runtime_counters_are_accepted() -> None:
    payload = {
        "id": "pii_filter", "name": "PII Filter", "description": "bounded inspection",
        "config": {"mode": "disable"},
        "runtime": dict.fromkeys(("event_count", "execution_count", "applied_count", "skipped_count", "total_duration_us", "max_duration_us", "detection_count", "block_count", "rewrite_count"), 0),
    }
    payload["runtime"].update(enabled=False, brokered_credentials=[])
    validate_case(RouteCase("GET", "/plugins/{plugin_id}/info", "pii_filter"), payload, None)
