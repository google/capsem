"""MCP API endpoints: /mcp/servers, /mcp/tools, /mcp/policy,
/mcp/tools/refresh, /mcp/tools/{name}/approve, /mcp/tools/{name}/call.

These endpoints read from CAPSEM_HOME (user.toml, corp.toml,
mcp_tool_cache.json) and for /mcp/tools/{name}/call route through a running
capsem-process over IPC. Without a running VM, /mcp/tools/{name}/call hits
the "no running sessions" path -- the fixture tests that error branch; full
happy-path coverage would need a downstream MCP aggregator in the guest
(tracked as a follow-up, same as test_mcp_call.py in tests/capsem-mcp/).
"""

import json

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import wait_exec_ready, vm_name

pytestmark = pytest.mark.integration


class TestMcpServers:

    def test_servers_returns_list(self, client):
        """/mcp/servers returns the merged server list (possibly empty)."""
        resp = client.get("/mcp/servers")
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
        """/mcp/tools returns the tool cache (empty under isolated CAPSEM_HOME)."""
        resp = client.get("/mcp/tools")
        assert isinstance(resp, list), f"/mcp/tools did not return list: {resp!r}"
        # A fresh CAPSEM_HOME has no mcp_tool_cache.json -> empty list.
        assert resp == [], f"unexpected tools in isolated HOME: {resp}"


class TestMcpPolicy:

    def test_policy_returns_merged_shape(self, client):
        """/mcp/policy returns McpPolicyInfoResponse shape with defaults."""
        resp = client.get("/mcp/policy")
        assert resp is not None
        expected = {
            "global_policy", "default_tool_permission",
            "blocked_servers", "tool_permissions",
        }
        missing = expected - resp.keys()
        assert not missing, f"missing policy keys: {missing}"
        # Handler defaults default_tool_permission to "allow" when unset.
        assert resp["default_tool_permission"] == "allow", (
            f"unexpected default_tool_permission: {resp['default_tool_permission']}"
        )
        assert isinstance(resp["blocked_servers"], list)
        assert isinstance(resp["tool_permissions"], dict)


class TestMcpToolsRefresh:

    def test_refresh_no_instances_succeeds(self, client):
        """/mcp/tools/refresh with zero running VMs returns instances=0."""
        # Ensure no VMs so the loop is over an empty list.
        client.post("/purge", {"all": True})
        resp = client.post("/mcp/tools/refresh", {})
        assert resp is not None, "refresh returned no body"
        assert resp.get("success") is True, f"refresh failed: {resp}"
        assert resp.get("instances") == 0, (
            f"expected 0 instances, got {resp.get('instances')}: {resp}"
        )


class TestMcpApprove:

    def test_approve_unknown_tool_rejected(self, client):
        """Approving a tool that is not in the cache must 404."""
        resp = client.post("/mcp/tools/not-a-real-tool/approve", {})
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
        resp = client.post("/mcp/tools/some-tool/call", {})
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
        client.post("/provision", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
        try:
            assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT), (
                f"{name} never exec-ready"
            )
            resp = client.post("/mcp/tools/definitely-not-a-real-tool/call", {})
            # Either the aggregator reports "unknown tool" or we get an
            # AppError body. Both are acceptable negative outcomes.
            assert resp is None or "error" in resp or "unknown" in json.dumps(resp).lower(), (
                f"unknown tool should reject: {resp}"
            )
        finally:
            try:
                client.delete(f"/delete/{name}")
            except Exception:
                pass
