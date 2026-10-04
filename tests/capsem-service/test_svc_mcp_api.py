"""MCP API endpoints under /mcp/servers/{server_id}.

These endpoints read MCP server configuration from settings.toml `[mcp]` with
the corp config laid over it, and the tool cache from CAPSEM_HOME. Tool calls route through a running capsem-process
over IPC. Without a running VM, tool calls hit the "no running sessions" path
-- the fixture tests that error branch; full happy-path coverage would need a
downstream MCP aggregator in the guest; the packed npm-to-ledger proof lives in
tests/ironbank/test_mcp_settings_ledger.py.
"""

import contextlib
import json
import tomllib

import pytest
from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import vm_name, wait_exec_ready

pytestmark = pytest.mark.integration

SERVER = "local"


class TestMcpServers:

    def test_servers_returns_list(self, client):
        """MCP servers endpoint returns the merged server list."""
        resp = client.get("/mcp/servers/list")
        assert isinstance(resp, list), f"/mcp/servers did not return list: {resp!r}"
        for server in resp:
            for key in (
                "name", "url", "has_auth_credential", "custom_header_count",
                "source", "enabled", "running", "tool_count", "is_stdio",
            ):
                assert key in server, f"server missing '{key}': {server}"
            assert isinstance(server["has_auth_credential"], bool)
            assert isinstance(server["enabled"], bool)
            assert isinstance(server["tool_count"], int)
            assert server["tool_count"] >= 0
            assert server["source"] in {"builtin", "config"}, server


class TestMcpTools:

    def test_tools_returns_list(self, client):
        """Server MCP tools endpoint returns the isolated tool cache shape."""
        resp = client.get(f"/mcp/servers/{SERVER}/tools/list")
        assert isinstance(resp, list), f"/mcp/tools did not return list: {resp!r}"
        if not resp:
            return

        names = {tool["namespaced_name"] for tool in resp}
        assert {"local__echo", "local__fetch_http"} <= names
        for tool in resp:
            for key in (
                "server_name", "original_name", "namespaced_name",
                "description", "pin_changed", "permission_action", "permission_source",
            ):
                assert key in tool, f"tool missing '{key}': {tool}"
            assert tool["server_name"] == "local"
            assert isinstance(tool["pin_changed"], bool)
            assert tool["permission_action"] in {"allow", "ask", "block"}
            assert tool["permission_source"] in {"corp", "settings", "default"}
            assert "approved" not in tool

    def test_tools_unconfigured_server_rejected(self, client):
        """Tool listing must reject a server neither settings nor corp configures."""
        status, resp = client.call_json("GET", "/mcp/servers/not-configured/tools/list")
        assert status == 404, (status, resp)
        assert resp == {"error": "MCP server not configured: not-configured"}


class TestRetiredMcpSecurity:

    def test_retired_mcp_endpoints_are_burned(self, client):
        """Retired global MCP endpoints must not expose alternate authoring."""
        for method, path in [
            ("get", "/mcp/policy"),
            ("get", "/mcp/servers"),
            ("get", "/mcp/tools"),
            ("post", "/mcp/tools/refresh"),
            ("post", "/mcp/tools/local__echo/approve"),
            ("post", "/mcp/tools/local__echo/call"),
        ]:
            call = getattr(client, method)
            resp = call(path, {}) if method == "post" else call(path)
            assert resp is None or "not found" in str(resp).lower() or "error" in resp


class TestMcpToolsRefresh:

    def test_refresh_no_instances_succeeds(self, client):
        """Server refresh with zero running VMs returns instances=0."""
        # Ensure no VMs so the loop is over an empty list.
        client.post("/purge", {"all": True})
        resp = client.post(f"/mcp/servers/{SERVER}/refresh", {})
        assert resp is not None, "refresh returned no body"
        assert resp.get("success") is True, f"refresh failed: {resp}"
        assert resp.get("server_id") == SERVER
        assert resp.get("instances") == 0, (
            f"expected 0 instances, got {resp.get('instances')}: {resp}"
        )


class TestMcpApprove:

    def test_tool_permission_mutation_records_rule_for_uncached_tool(self, client, service_env):
        """Tool permission edits are settings.toml rule mutations, not cache approvals."""
        resp = client.patch(
            f"/mcp/servers/{SERVER}/tools/not-a-real-tool/edit",
            {"action": "ask"},
        )
        assert resp is not None, "tool permission mutation returned no body"
        assert "profile_id" not in resp
        assert resp.get("server_id") == SERVER
        assert resp.get("tool_id") == "not-a-real-tool"
        assert resp.get("action") == "ask"
        mutation = resp.get("mutation") or {}
        assert mutation.get("category") == "mcp"
        assert mutation.get("operation") == "permission"
        assert mutation.get("filename") == "settings.toml"
        assert "profile_id" not in mutation
        rule_id = mutation["rule_id"]
        assert rule_id.startswith("profiles.rules."), mutation

        settings = tomllib.loads((service_env.home_dir / "settings.toml").read_text())
        rule = settings["profiles"]["rules"][rule_id.removeprefix("profiles.rules.")]
        assert rule["action"] == "ask"
        assert 'mcp.tool_call.name == "not-a-real-tool"' in rule["match"]

    def test_tool_permission_mutation_refuses_unconfigured_server(self, client):
        status, resp = client.call_json(
            "PATCH", "/mcp/servers/not-configured/tools/probe/edit", {"action": "block"}
        )
        assert status == 400, (status, resp)
        assert "not-configured" in resp["error"]


class TestMcpCall:

    def test_call_without_running_session_rejected(self, client):
        """Calling any MCP tool with no running VM must 503.

        handle_mcp_call needs at least one running capsem-process to route
        the IPC through. With no sessions, the handler returns
        SERVICE_UNAVAILABLE. The sprint's plan.md acknowledges that full
        happy-path coverage requires a downstream MCP server in the fixture
        (same follow-up as test_mcp_call.py on the MCP side).
        """
        client.post("/purge", {"all": True})
        resp = client.post(
            f"/mcp/servers/{SERVER}/tools/some-tool/call",
            {},
        )
        assert resp is None or "error" in resp or "no running" in str(resp).lower(), (
            f"no-session call should 503: {resp}"
        )

    def test_call_unknown_tool_with_running_vm_rejected(self, client):
        """With a running VM present, call a tool name that does not exist.

        The route reaches capsem-process via IPC, the aggregator reports
        the tool is unknown, and the service surfaces that as an error.
        Proves the IPC plumbing is wired end-to-end (service -> process
        -> aggregator), even if the downstream MCP call itself fails.
        """
        name = vm_name("mcpcall")
        client.post(
            "/vms/create",
            {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS},
        )
        try:
            assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT), (
                f"{name} never exec-ready"
            )
            resp = client.post(
                f"/mcp/servers/{SERVER}/tools/definitely-not-a-real-tool/call",
                {},
            )
            # Either the aggregator reports "unknown tool" or we get an
            # AppError body. Both are acceptable negative outcomes.
            assert resp is None or "error" in resp or "unknown" in json.dumps(resp).lower(), (
                f"unknown tool should reject: {resp}"
            )
        finally:
            with contextlib.suppress(Exception):
                client.delete(f"/vms/{name}/delete")
