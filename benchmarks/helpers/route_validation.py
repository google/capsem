"""Untimed preflight: canonical public schemas and successful fixture semantics.

The recorded corpus populates traffic/history; security and approval queues may
legitimately be empty. Idle queues do not certify pending-approval performance.
"""
from __future__ import annotations

import json
import sys
from ipaddress import IPv4Network
from pathlib import Path

# Use the selected source contract without installing another SDK cohort.
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "sdk/python"))

from capsem.models.container_status_response import ContainerStatusResponse
from capsem.models.file_list_response import FileListResponse
from capsem.models.history_response import HistoryResponse
from capsem.models.host_logs_response import HostLogsResponse
from capsem.models.logs_response import LogsResponse
from capsem.models.mcp_default_permission_response import McpDefaultPermissionResponse
from capsem.models.mcp_info_response import McpInfoResponse
from capsem.models.mcp_servers_list_response import McpServersListResponse
from capsem.models.mcp_tools_list_response import McpToolsListResponse
from capsem.models.network_info import NetworkInfo
from capsem.models.sandbox_info import SandboxInfo
from capsem.models.security_ask_list_response import SecurityAskListResponse
from capsem.models.timeline_response import TimelineResponse
from capsem.models.vm_stats_detail_response import VmStatsDetailResponse
from capsem.models.vm_stats_summary_response import VmStatsSummaryResponse
from helpers.stopped_workspace import VM_ID
from pydantic import TypeAdapter

from .route_cases import RouteCase
from .route_fixtures import MARKER, corpus_config

# Private UI routes are intentionally not all in selected OpenAPI. Their
# explicit field/counter contracts below come from the service DTOs.
SCHEMAS = {path: TypeAdapter(model) for path, model in {
    "/networks/{id}": NetworkInfo,
    "/vms/{id}/files/list": FileListResponse,
    "/vms/{id}/logs": LogsResponse,
    "/host-logs/{name}": HostLogsResponse,
    "/mcp/servers/{server_id}/tools/list": McpToolsListResponse,
    "/vms/{id}/info": SandboxInfo,
    "/vms/{id}/stats/summary": VmStatsSummaryResponse,
    "/vms/{id}/stats/detail": VmStatsDetailResponse,
    "/vms/{id}/history": HistoryResponse,
    "/vms/{id}/timeline": TimelineResponse,
    "/vms/{id}/container": ContainerStatusResponse,
    "/vms/{id}/security/asks": SecurityAskListResponse,
    "/mcp/info": McpInfoResponse,
    "/mcp/servers/list": McpServersListResponse,
    "/mcp/default/info": McpDefaultPermissionResponse,
}.items()}


def count(value: object) -> int:
    assert type(value) is int and value >= 0, "expected nonnegative integer counter"
    return value


def security_stats(payload: dict) -> None:
    total = count(payload["total"])
    for key in ("by_action", "by_event_type", "by_level", "by_rule"):
        assert isinstance(payload[key], list)
        assert sum(count(row["count"]) for row in payload[key]) == total


def plugin_info(payload: dict, expected: str | None = None) -> None:
    assert isinstance(payload["id"], str) and payload["id"]
    if expected is not None:
        assert payload["id"] == expected
    assert payload["name"] and payload["description"]
    assert payload["config"]["mode"] in {"disable", "allow", "ask", "block", "rewrite"}
    runtime = payload["runtime"]
    assert type(runtime["enabled"]) is bool
    for key in ("event_count", "execution_count", "applied_count", "skipped_count", "total_duration_us", "max_duration_us", "detection_count", "block_count", "rewrite_count"):
        count(runtime[key])
    assert isinstance(runtime["brokered_credentials"], list)


def validate_case(case: RouteCase, payload, live_id: str | None, network_id: str | None = None) -> None:
    """Reject a fast empty/error response before any timed request."""
    path = case.path
    if path == "/networks/{id}":
        assert network_id is not None and payload["id"] == network_id
        assert payload["name"] == corpus_config()["network_name"] and payload["members"] == []
        assert count(payload["created_unix_ms"]) > 0 and IPv4Network(payload["subnet"]).is_private
    elif path == "/vms/{id}/files/list":
        cfg = corpus_config()
        names = {f"unchanged-{index:03}.txt" for index in range(cfg["workspace_files"])} | {"modified.txt", "created.txt"}
        assert {entry["path"] for entry in payload["entries"]} == names
        assert len(payload["entries"]) == len(names)
        assert all(entry["type"] == "file" and entry["size"] > 0 for entry in payload["entries"])
    elif path == "/vms/{id}/logs":
        assert payload["logs"] == payload["serial_logs"]
        for key in ("serial_logs", "process_logs"):
            validate_log(payload[key], tail=True)
    elif path == "/host-logs/{name}":
        assert payload["source"] == case.parameters["name"]
        validate_log(payload["text"], tail=True)
    elif path == "/mcp/servers/{server_id}/tools/list":
        names = corpus_config()["mcp_tools"]
        assert [tool["original_name"] for tool in payload] == names
        assert all(tool["server_name"] == "local" and tool["namespaced_name"] == "local__" + tool["original_name"] for tool in payload)
    elif path == "/vms/{id}/info":
        assert payload["id"] == VM_ID and payload["persistent"] is True
        assert payload["session_db"]["ready"] is True
        assert count(payload["total_requests"]) > 0
    elif path.endswith(("/save/status", "/fork/status")):
        assert payload["vm_id"] == VM_ID
        assert payload["operation"] == path.split("/")[-2]
        assert payload["status"] == "idle" and payload["in_progress"] is False
        assert payload["message"]
    elif path == "/vms/{id}/stats/summary":
        assert count(payload["total_requests"]) > 0
        assert count(payload["total_input_tokens"]) > 0
        assert count(payload["allowed_requests"]) + count(payload["denied_requests"]) == payload["total_requests"]
    elif path == "/vms/{id}/stats/detail":
        assert payload["http_events"] and payload["model_events"]
        assert payload["body_blobs"], "real body archive must be read"
    elif path == "/vms/{id}/timeline":
        assert payload["events"], "recorded timeline must not be empty"
    elif path == "/vms/{id}/history":
        assert payload["commands"] and count(payload["total"]) >= len(payload["commands"])
        assert type(payload["has_more"]) is bool
    elif path.endswith("/history/counts"):
        assert count(payload["exec_count"]) + count(payload["audit_count"]) > 0
    elif path.endswith("/history/processes"):
        assert isinstance(payload["processes"], list) and payload["processes"]
    elif path.endswith(("/security/latest", "/detection/latest", "/enforcement/latest")):
        assert isinstance(payload, list)
        for row in payload:
            event = row["event"] if "{id}" not in path else row
            assert event["event_id"] and type(event["timestamp_unix_ms"]) is int
    elif path.endswith(("/detection/status", "/enforcement/status")) and "{id}" in path:
        security_stats(payload)
    elif path == "/security/status":
        assert count(payload["total"]) == sum(count(row["stats"]["total"]) for row in payload["sessions"])
        assert VM_ID in {row["vm_id"] for row in payload["sessions"]}
        for row in payload["sessions"]:
            security_stats(row["stats"])
    elif path == "/plugins/list":
        assert isinstance(payload["plugins"], list) and payload["plugins"]
        for plugin in payload["plugins"]:
            plugin_info(plugin)
    elif path == "/plugins/{plugin_id}/info":
        plugin_info(payload, case.case)
    elif path == "/plugins/credential_broker/credentials/info":
        assert payload["plugin_id"] == "credential_broker"
        assert type(payload["store"]["ready"]) is bool
        for key in ("inventory", "sessions", "corp_constraints"):
            assert isinstance(payload[key], list)
        assert type(payload["grants"]["enabled"]) is bool
    elif path == "/settings/info":
        assert payload["tree"] and isinstance(payload["issues"], list)
    elif path == "/corp/info":
        assert type(payload["installed"]) is bool and isinstance(payload["paths"], list)
        assert all(type(row["exists"]) is bool and row["path"] for row in payload["paths"])
    elif path == "/mcp/info":
        assert count(payload["server_count"]) == count(payload["manual_server_count"]) + int(payload["builtin_local_enabled"])
    elif path == "/mcp/servers/list":
        assert isinstance(payload, list)
    elif path == "/mcp/default/info":
        assert payload["action"] in {"allow", "ask", "block", "preprocess", "rewrite", "postprocess"}
        assert payload["source"]
    elif path == "/vms/{id}/container":
        assert payload["image"] and payload["state"] == "running" and not payload.get("error")
    elif path == "/vms/{id}/security/asks":
        assert payload["vm_id"] == live_id and payload["generation"]
        assert count(payload["pending_count"]) == 0 and payload["requests"] == []


def validate_log(text: str, *, tail: bool) -> None:
    cfg = corpus_config()
    assert isinstance(text, str) and len(text.encode()) <= cfg["log_read_bytes"]
    lines = text.splitlines()
    matches = [line for line in lines if line.startswith(MARKER)]
    assert len(matches) >= cfg["log_tail"]
    if tail:
        assert len(matches) == len(lines) == cfg["log_tail"]
    assert len(set(matches)) == len(matches)


def validate_wire(case: RouteCase, body: bytes, live_id: str | None, network_id: str | None = None) -> object:
    if case.response == "text":
        assert case.path == "/service-logs", "unreviewed text contract"
        text = body.decode("utf-8")
        validate_log(text, tail=False)
        return text
    if schema := SCHEMAS.get(case.path):
        schema.validate_json(body)
    payload = json.loads(body)
    validate_case(case, payload, live_id, network_id)
    return payload


def validate_snapshot(payloads: dict) -> None:
    """The same stopped ledger must yield the same counters across read models."""
    info = payloads.get("/vms/{id}/info")
    summary = payloads.get("/vms/{id}/stats/summary")
    if info is not None and summary is not None:
        for key in ("total_requests", "total_input_tokens", "total_output_tokens"):
            assert count(info[key]) == count(summary[key])
        assert summary["total_requests"] > 0
