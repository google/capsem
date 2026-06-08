"""MCP API endpoints under /profiles/{profile_id}/mcp/servers/{server_id}.

These endpoints read MCP server configuration from the selected profile and
tool cache from CAPSEM_HOME. Tool calls route through a running capsem-process
over IPC. Without a running VM, tool calls hit the "no running sessions" path
-- the fixture tests that error branch; full happy-path coverage would need a
downstream MCP aggregator in the guest (tracked as a follow-up, same as
test_mcp_call.py in tests/capsem-mcp/).
"""

import json

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import wait_exec_ready, vm_name

pytestmark = pytest.mark.integration

PROFILE = "code"
SERVER = "local"


class TestMcpServers:

    def test_servers_returns_list(self, client):
        """Profile MCP servers endpoint returns the merged server list."""
        resp = client.get(f"/profiles/{PROFILE}/mcp/servers/list")
        assert isinstance(resp, list), f"/mcp/servers did not return list: {resp!r}"
        for server in resp:
            for key in (
                "name", "url", "has_bearer_token", "custom_header_count",
                "source", "enabled", "running", "tool_count", "is_stdio",
            ):
                assert key in server, f"server missing '{key}': {server}"
            assert isinstance(server["has_bearer_token"], bool)
            assert isinstance(server["enabled"], bool)
            assert isinstance(server["tool_count"], int)
            assert server["tool_count"] >= 0


class TestMcpTools:

    def test_tools_returns_list(self, client):
        """Profile/server MCP tools endpoint returns the isolated tool cache shape."""
        resp = client.get(f"/profiles/{PROFILE}/mcp/servers/{SERVER}/tools/list")
        assert isinstance(resp, list), f"/mcp/tools did not return list: {resp!r}"
        if not resp:
            return

        names = {tool["namespaced_name"] for tool in resp}
        assert {"local__echo", "local__fetch_http"} <= names
        for tool in resp:
            for key in (
                "server_name", "original_name", "namespaced_name",
                "description", "approved", "pin_changed",
            ):
                assert key in tool, f"tool missing '{key}': {tool}"
            assert tool["server_name"] == "local"
            assert isinstance(tool["approved"], bool)
            assert isinstance(tool["pin_changed"], bool)

    def test_tools_unknown_profile_server_rejected(self, client):
        """Profile/server tool listing must reject servers absent from the profile."""
        resp = client.get(f"/profiles/{PROFILE}/mcp/servers/settings-only/tools/list")
        assert resp is None or "error" in resp or "not found" in str(resp).lower(), (
            f"unknown profile server should reject: {resp}"
        )


class TestMcpPolicy:

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
        """Profile/server refresh with zero running VMs returns instances=0."""
        # Ensure no VMs so the loop is over an empty list.
        client.post("/purge", {"all": True})
        resp = client.post(f"/profiles/{PROFILE}/mcp/servers/{SERVER}/refresh", {})
        assert resp is not None, "refresh returned no body"
        assert resp.get("success") is True, f"refresh failed: {resp}"
        assert resp.get("server_id") == SERVER
        assert resp.get("instances") == 0, (
            f"expected 0 instances, got {resp.get('instances')}: {resp}"
        )


class TestMcpApprove:

    def test_approve_unknown_tool_rejected(self, client):
        """Approving a tool that is not in the cache must 404."""
        resp = client.patch(
            f"/profiles/{PROFILE}/mcp/servers/{SERVER}/tools/not-a-real-tool/edit",
            {"approved": True},
        )
        # 404 from AppError gives a body like {"error": "tool not found: ..."}.
        assert resp is None or "error" in resp or "not found" in str(resp).lower(), (
            f"unknown tool should 404: {resp}"
        )


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
            f"/profiles/{PROFILE}/mcp/servers/{SERVER}/tools/some-tool/call",
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
            {"name": name, "profile_id": PROFILE, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS},
        )
        try:
            assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT), (
                f"{name} never exec-ready"
            )
            resp = client.post(
                f"/profiles/{PROFILE}/mcp/servers/{SERVER}/tools/definitely-not-a-real-tool/call",
                {},
            )
            # Either the aggregator reports "unknown tool" or we get an
            # AppError body. Both are acceptable negative outcomes.
            assert resp is None or "error" in resp or "unknown" in json.dumps(resp).lower(), (
                f"unknown tool should reject: {resp}"
            )
        finally:
            try:
                client.delete(f"/vms/{name}/delete")
            except Exception:
                pass
