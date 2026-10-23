"""Bounded real route-read corpus and strict response contracts."""
from __future__ import annotations

import json
from types import SimpleNamespace

import pytest
from helpers.stopped_workspace import VM_ID

from benchmarks.helpers.route_cases import RouteCase
from benchmarks.helpers.route_fixtures import corpus_config, seed_logs, seed_metadata
from benchmarks.helpers.route_validation import validate_wire


def test_seeded_corpus_is_exact_bounded_and_never_replaces_active_logs(tmp_path) -> None:
    service = SimpleNamespace(home_dir=tmp_path / "home", tmp_dir=tmp_path / "run")
    service.home_dir.mkdir(); service.tmp_dir.mkdir()
    (service.tmp_dir / "service.log").write_text("live owner\n")
    session = service.tmp_dir / "persistent" / VM_ID
    session.mkdir(parents=True)
    cfg = corpus_config()
    seed_metadata(service, cfg)
    seed_logs(service, cfg)
    assert (service.tmp_dir / "service.log").read_text() == "live owner\n"
    for path in (session / "serial.log", session / "process.log", service.tmp_dir / "mcp.log", service.tmp_dir / "service.benchmark.log"):
        assert path.stat().st_size == cfg["log_bytes"]
        lines = path.read_text().splitlines()
        assert len(lines) == cfg["log_bytes"] // cfg["log_line_bytes"]
        assert len(set(lines)) == len(lines)
    tools = json.loads((service.home_dir / "mcp_tool_cache.json").read_text())
    assert [tool["original_name"] for tool in tools] == cfg["mcp_tools"]
    assert all(tool["approved"] is False for tool in tools)


def test_read_cases_resolve_explicit_query_bounds() -> None:
    case = RouteCase("GET", "/host-logs/{name}", "mcp-tail", parameters={"name": "mcp"}, query={"tail": 16, "max_bytes": 102400})
    assert case.resolve(None) == "/host-logs/mcp?tail=16&max_bytes=102400"
    assert case.metric_key != RouteCase("GET", case.path, "service-tail").metric_key


def test_service_logs_preserve_text_wire_contract() -> None:
    case = RouteCase("GET", "/service-logs", "retained-text", response="text")
    cfg = corpus_config()
    text = b"".join(f"CAPSEM-ROUTE-CORPUS {i:08d}\n".encode() for i in range(cfg["log_tail"]))
    assert validate_wire(case, text, None) == text.decode()
    with pytest.raises((AssertionError, ValueError)):
        validate_wire(case, b"{}", None)
    with pytest.raises(UnicodeDecodeError):
        validate_wire(case, b"\xff", None)


def test_empty_file_list_and_wrong_cached_tools_are_refused() -> None:
    for case, body in [
        (RouteCase("GET", "/vms/{id}/files/list", "seeded-workspace"), b'{"entries":[]}'),
        (RouteCase("GET", "/mcp/servers/{server_id}/tools/list", "local-cache"), b'[]'),
    ]:
        with pytest.raises(AssertionError):
            validate_wire(case, body, None)


def test_query_limits_and_corpus_expectations_are_one_configured_cohort() -> None:
    from benchmarks.helpers.route_cases import measured_cases

    cfg = corpus_config()
    cases = measured_cases()
    for case in cases:
        if case.path in {"/vms/{id}/logs", "/host-logs/{name}"}:
            assert case.query["tail"] == cfg["log_tail"]
            assert case.query["max_bytes"] == cfg["log_read_bytes"]
            assert case.query["grep"] == "CAPSEM-ROUTE-CORPUS"
        if case.path == "/vms/{id}/files/list":
            assert case.query["depth"] == cfg["file_depth"]
    assert cfg["log_bytes"] > cfg["log_read_bytes"]


def test_log_seed_refuses_to_replace_existing_stream_generation(tmp_path) -> None:
    service = SimpleNamespace(tmp_dir=tmp_path)
    session = tmp_path / "persistent" / VM_ID
    session.mkdir(parents=True)
    (session / "serial.log").write_text("existing owner")
    with pytest.raises(AssertionError, match="refusing to replace"):
        seed_logs(service, corpus_config())
    assert (session / "serial.log").read_text() == "existing owner"
