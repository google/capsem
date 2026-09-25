"""Guest MCP integration tests.

Verifies that the capsem-mcp-server binary exists and that the host
MITM MCP endpoint responds to JSON-RPC messages over framed vsock:5002.
"""

import json
import os
import re
import subprocess

import pytest

from .diagnostic_support import run

LOCAL_MOCK_SERVER_ENV = "CAPSEM_MOCK_SERVER_BASE_URL"
SECRET_PATTERN = re.compile(
    r"(sk-[A-Za-z0-9_-]{20,}|ghp_[A-Za-z0-9_]{20,}|AIza[0-9A-Za-z_-]{20,})"
)


def _local_mock_url(path):
    base_url = os.environ.get(LOCAL_MOCK_SERVER_ENV)
    if not base_url:
        return None
    return f"{base_url.rstrip('/')}/{path.lstrip('/')}"


def _require_local_mock_url(path, reason):
    url = _local_mock_url(path)
    if not url:
        pytest.fail(f"{reason}; set {LOCAL_MOCK_SERVER_ENV}")
    return url


# ---------------------------------------------------------------------------
# Helper
# ---------------------------------------------------------------------------

def _mcp_call(messages, timeout=15):
    """Send JSON-RPC messages to capsem-mcp-server, collect responses.

    capsem-mcp-server connects to the host MITM MCP endpoint on
    vsock:5002 and relays stdio JSON-RPC over framed MCP records. We
    send messages on stdin and read responses from stdout.
    """
    input_lines = "\n".join(json.dumps(m) for m in messages) + "\n"
    proc = subprocess.run(
        ["/run/capsem-mcp-server"],
        input=input_lines,
        capture_output=True,
        text=True,
        timeout=timeout,
    )
    assert proc.returncode == 0, (
        f"capsem-mcp-server exited {proc.returncode}: {proc.stderr}"
    )
    responses = []
    for line in proc.stdout.strip().splitlines():
        line = line.strip()
        if line:
            responses.append(json.loads(line))
    assert len(responses) > 0, (
        f"capsem-mcp-server returned no responses (stderr: {proc.stderr})"
    )
    return responses


def _mcp_raw(input_text, timeout=15):
    """Send raw stdin to capsem-mcp-server and collect JSON responses."""
    proc = subprocess.run(
        ["/run/capsem-mcp-server"],
        input=input_text,
        capture_output=True,
        text=True,
        timeout=timeout,
    )
    assert proc.returncode == 0, (
        f"capsem-mcp-server exited {proc.returncode}: {proc.stderr}"
    )
    responses = []
    for line in proc.stdout.strip().splitlines():
        line = line.strip()
        if line:
            responses.append(json.loads(line))
    assert len(responses) > 0, (
        f"capsem-mcp-server returned no responses (stderr: {proc.stderr})"
    )
    return responses


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------

def test_mcp_server_binary_exists():
    """capsem-mcp-server binary is installed and executable."""
    r = run("test -x /run/capsem-mcp-server && echo ok")
    assert "ok" in r.stdout


def test_mcp_initialize():
    """MCP initialize handshake returns serverInfo."""
    responses = _mcp_call([
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "capsem-doctor", "version": "1.0"},
            },
        },
    ])
    assert len(responses) >= 1
    resp = responses[0]
    assert resp.get("id") == 1
    assert "result" in resp
    assert resp["result"]["serverInfo"]["name"] == "capsem-mcp-mitm-endpoint"


def test_mcp_tools_list():
    """tools/list returns the three built-in HTTP tools."""
    responses = _mcp_call([
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "capsem-doctor", "version": "1.0"},
            },
        },
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/list"},
    ])
    # Find the tools/list response (id=2).
    tools_resp = [r for r in responses if r.get("id") == 2]
    assert len(tools_resp) == 1
    tools = tools_resp[0]["result"]["tools"]
    names = [t["name"] for t in tools]
    assert "local__fetch_http" in names
    assert "local__grep_http" in names
    assert "local__http_headers" in names


def test_mcp_invalid_json_recovers():
    """Malformed JSON gets a parse error and the same bridge still works."""
    init = json.dumps({
        "jsonrpc": "2.0",
        "id": "after-bad-json",
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "capsem-doctor", "version": "1.0"},
        },
    })
    responses = _mcp_raw("{not json\n" + init + "\n")
    parse_errors = [
        r for r in responses
        if "id" not in r and r.get("error", {}).get("code") == -32700
    ]
    assert len(parse_errors) == 1
    init_resp = [r for r in responses if r.get("id") == "after-bad-json"]
    assert len(init_resp) == 1
    assert init_resp[0]["result"]["serverInfo"]["name"] == "capsem-mcp-mitm-endpoint"


def test_mcp_notification_interleaving_has_no_responses():
    """Notifications interleaved between requests must stay fire-and-forget."""
    responses = _mcp_call([
        {
            "jsonrpc": "2.0",
            "id": "notify-init",
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "capsem-doctor", "version": "1.0"},
            },
        },
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {
            "jsonrpc": "2.0",
            "method": "$/progress",
            "params": {"progressToken": "doctor", "progress": 1, "total": 2},
        },
        {"jsonrpc": "2.0", "id": "notify-tools", "method": "tools/list"},
    ])
    assert {r.get("id") for r in responses} == {"notify-init", "notify-tools"}


def test_mcp_oversized_request_returns_local_error_and_recovers():
    """Oversized guest requests fail locally and do not poison the relay."""
    responses = _mcp_call([
        {
            "jsonrpc": "2.0",
            "id": "doctor-too-big",
            "method": "tools/call",
            "params": {
                "name": "local__echo",
                "arguments": {"text": "x" * 1100000},
            },
        },
        {
            "jsonrpc": "2.0",
            "id": "doctor-after-too-big",
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "capsem-doctor", "version": "1.0"},
            },
        },
    ], timeout=20)
    by_id = {r["id"]: r for r in responses if "id" in r}
    assert by_id["doctor-too-big"]["error"]["code"] == -32001
    assert "frame encode failed" in by_id["doctor-too-big"]["error"]["message"]
    assert by_id["doctor-after-too-big"]["result"]["serverInfo"]["name"] == (
        "capsem-mcp-mitm-endpoint"
    )


def test_mcp_large_last_request_before_stdin_closes_is_answered():
    """A client that sends a large final request and closes stdin at once
    still gets every response.

    The relay ends the session when stdin closes. On Apple VZ a vsock
    shutdown can reach the host ahead of bytes still in flight, and a frame
    cut short by that end is a connection error on the host, so the last
    request would get no answer. Several rounds, because the loss is a race.
    """
    initialize = {
        "protocolVersion": "2024-11-05",
        "capabilities": {},
        "clientInfo": {"name": "capsem-doctor", "version": "1.0"},
    }
    for round_number in range(5):
        last = f"doctor-last-{round_number}"
        messages: list[dict[str, object]] = [
            {"jsonrpc": "2.0", "id": f"doctor-{round_number}-{i}", "method": "tools/list"}
            for i in range(3)
        ]
        messages.append(
            {
                "jsonrpc": "2.0",
                "id": last,
                "method": "initialize",
                "params": {**initialize, "padding": "x" * 900_000},
            }
        )
        responses = _mcp_call(messages, timeout=30)
        answered = {r["id"] for r in responses if "id" in r}
        expected = {str(m["id"]) for m in messages}
        assert answered == expected, (
            f"round {round_number}: unanswered {sorted(expected - answered)}"
        )


def test_mcp_fetch_http_allowed_domain():
    """fetch_http on the local mock server succeeds."""
    url = _require_local_mock_url("/tiny", "local MCP fetch_http smoke")
    responses = _mcp_call([
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "capsem-doctor", "version": "1.0"},
            },
        },
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "local__fetch_http",
                "arguments": {"url": url, "max_length": 1000},
            },
        },
    ])
    call_resp = [r for r in responses if r.get("id") == 3]
    assert len(call_resp) == 1
    result = call_resp[0]["result"]
    assert result.get("isError") is not True
    content_text = result["content"][0]["text"]
    assert f"URL: {url}" in content_text
    assert "capsem-mock-server:tiny" in content_text


def test_mcp_fetch_http_blocked_domain():
    """fetch_http on a blocked domain returns isError."""
    result = _init_and_call(
        "fetch_http",
        {"url": "https://evil-blocked-domain.xyz"},
    )
    assert result.get("isError") is True, f"expected isError: {result}"
    assert "blocked" in result["content"][0]["text"].lower(), (
        f"expected 'blocked' in error text: {result}"
    )


# Every tool served through the guest MCP endpoint is namespaced with its source
# server's key. The built-in tools all live behind the ``local`` server
# (see ``config/defaults.json``'s ``mcp.local`` entry and
# ``namespace_name`` in ``capsem-core/src/mcp/types.rs``), so their wire
# names are ``local__<tool>``. Tests take the bare name and this helper
# applies the prefix.
LOCAL_NS = "local__"


def ns(tool_name: str) -> str:
    """Return the namespaced wire name for a built-in local tool."""
    return f"{LOCAL_NS}{tool_name}" if not tool_name.startswith(LOCAL_NS) else tool_name


def _init_and_call(tool_name, arguments, call_id=10, timeout=15):
    """Helper: initialize + call a tool in one shot, return the result dict.

    ``tool_name`` is the bare tool identifier (e.g. ``fetch_http``); the
    endpoint's namespaced form (``local__fetch_http``) is applied here so
    callers stay readable.
    """
    wire_name = ns(tool_name)
    responses = _mcp_call([
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "capsem-doctor", "version": "1.0"},
            },
        },
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {
            "jsonrpc": "2.0",
            "id": call_id,
            "method": "tools/call",
            "params": {"name": wire_name, "arguments": arguments},
        },
    ], timeout=timeout)
    call_resp = [r for r in responses if r.get("id") == call_id]
    assert len(call_resp) == 1, f"expected 1 response for id={call_id}, got {len(call_resp)}"
    resp = call_resp[0]
    # Collapse a JSON-RPC protocol error into an ``isError: true`` tool result
    # so callers see a single shape. Aggregators emit JSON-RPC ``error`` when
    # the tool layer is out of spec (wrong name, missing required arg, policy
    # block); tools emit ``isError`` on the result when their own handler
    # rejects the input. Both signal "this call failed"; conflating them
    # keeps tests simple.
    if "error" in resp:
        err = resp["error"]
        code = err.get("code")
        message = err.get("message", "")
        return {
            "isError": True,
            "content": [{
                "type": "text",
                "text": f"[{code}] {message}" if code is not None else message,
            }],
        }
    return resp["result"]


# ---------------------------------------------------------------------------
# Content verification -- fetch_http must return real page text
# ---------------------------------------------------------------------------

def test_mcp_fetch_http_returns_real_content():
    """fetch_http returns actual local fixture content, not empty text."""
    url = _require_local_mock_url("/tiny", "local MCP fetch_http content smoke")
    result = _init_and_call(
        "fetch_http",
        {"url": url, "max_length": 5000},
    )
    assert result.get("isError") is not True, f"fetch failed: {result}"
    text = result["content"][0]["text"]
    assert "capsem-mock-server:tiny" in text, (
        f"fetch_http returned no real local fixture content: {text[:500]}"
    )


# ---------------------------------------------------------------------------
# Content verification -- grep_http positive match
# ---------------------------------------------------------------------------

def test_mcp_grep_http_finds_matches():
    """grep_http on the local mock server must find matches."""
    url = _require_local_mock_url("/html/about", "local MCP grep_http smoke")
    result = _init_and_call(
        "grep_http",
        {"url": url, "pattern": "Google"},
    )
    assert result.get("isError") is not True, f"grep failed: {result}"
    text = result["content"][0]["text"]
    assert "Matches found: 0" not in text, (
        f"grep_http found 0 matches on local fixture -- extraction broken: {text[:500]}"
    )
    assert "Match 1" in text, (
        f"grep_http output missing match blocks: {text[:500]}"
    )


# ---------------------------------------------------------------------------
# Negative tests -- blocked domains
# ---------------------------------------------------------------------------

def test_mcp_grep_http_blocked_domain():
    """grep_http on a blocked domain returns isError."""
    result = _init_and_call(
        "grep_http",
        {"url": "https://evil-blocked-domain.xyz", "pattern": "test"},
    )
    assert result["isError"] is True
    assert "blocked" in result["content"][0]["text"].lower()


def test_mcp_http_headers_blocked_domain():
    """http_headers on a blocked domain returns isError."""
    result = _init_and_call(
        "http_headers",
        {"url": "https://evil-blocked-domain.xyz"},
    )
    assert result["isError"] is True
    assert "blocked" in result["content"][0]["text"].lower()


# ---------------------------------------------------------------------------
# http_headers positive test
# ---------------------------------------------------------------------------

def test_mcp_http_headers_allowed_domain():
    """http_headers on the local mock server returns status and headers."""
    url = _require_local_mock_url("/tiny", "local MCP http_headers smoke")
    result = _init_and_call(
        "http_headers",
        {"url": url},
    )
    assert result.get("isError") is not True, f"http_headers failed: {result}"
    text = result["content"][0]["text"]
    assert "Status:" in text, f"missing status line: {text[:300]}"
    assert "content-type" in text.lower(), f"missing content-type header: {text[:500]}"


def test_claude_mcp_list_shows_capsem():
    """Claude sees the profile-owned Capsem MCP bridge."""
    r = run("claude mcp list 2>&1", timeout=15)
    assert r.returncode == 0, f"claude mcp list failed: {r.stderr}"
    assert "capsem:" in r.stdout, f"Claude MCP config missing capsem: {r.stdout}"
    assert "/run/capsem-mcp-server" in r.stdout, (
        f"Claude MCP bridge points at the wrong command: {r.stdout}"
    )
    assert "No MCP servers configured" not in r.stdout, (
        f"Claude ignored profile-owned MCP config: {r.stdout}"
    )


def test_claude_state_json_has_capsem_mcp():
    """Claude state is profile-owned trust state and must not embed MCP or secrets."""
    r = run("cat /root/.claude.json")
    assert r.returncode == 0, f"missing Claude profile state: {r.stderr}"
    assert not SECRET_PATTERN.search(r.stdout), "secret-like value found in Claude state"
    settings = json.loads(r.stdout)
    assert "mcpServers" not in settings or not settings["mcpServers"], (
        f"Claude state must not create a second MCP authority: {settings.get('mcpServers')}"
    )
    assert settings["hasTrustDialogAccepted"] is True
    assert settings["projects"]["/root"]["hasTrustDialogAccepted"] is True


def test_profile_mcp_registry_has_capsem_bridge_only():
    """The profile-owned MCP registry is the canonical MCP authority."""
    r = run("cat /root/.mcp.json")
    assert r.returncode == 0, f"missing canonical MCP registry: {r.stderr}"
    assert not SECRET_PATTERN.search(r.stdout), "secret-like value found in MCP registry"
    registry = json.loads(r.stdout)
    assert registry == {
        "mcpServers": {
            "capsem": {
                "command": "/run/capsem-mcp-server",
            },
        },
    }


def test_codex_config_has_capsem_mcp():
    """Codex config must consume the same profile-owned Capsem MCP bridge."""
    r = run("cat /root/.codex/config.toml")
    assert r.returncode == 0, f"missing Codex profile config: {r.stderr}"
    assert not SECRET_PATTERN.search(r.stdout), "secret-like value found in Codex config"
    assert '[mcp_servers.capsem]' in r.stdout
    assert 'command = "/run/capsem-mcp-server"' in r.stdout


def test_mcp_tools_list_has_descriptions():
    """Every tool in tools/list must have a non-empty description."""
    responses = _mcp_call([
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "capsem-doctor", "version": "1.0"},
            },
        },
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/list"},
    ])
    tools_resp = [r for r in responses if r.get("id") == 2]
    assert len(tools_resp) == 1
    tools = tools_resp[0]["result"]["tools"]
    for tool in tools:
        desc = tool.get("description", "")
        assert desc and len(desc) > 10, (
            f"tool '{tool['name']}' has missing or trivial description: {desc!r}"
        )


def test_mcp_tools_list_has_input_schema():
    """Every tool in tools/list must have a valid inputSchema."""
    responses = _mcp_call([
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "capsem-doctor", "version": "1.0"},
            },
        },
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/list"},
    ])
    tools_resp = [r for r in responses if r.get("id") == 2]
    tools = tools_resp[0]["result"]["tools"]
    for tool in tools:
        schema = tool.get("inputSchema")
        assert schema is not None, f"tool '{tool['name']}' missing inputSchema"
        assert schema.get("type") == "object", (
            f"tool '{tool['name']}' inputSchema type should be 'object', got {schema.get('type')!r}"
        )
        assert "properties" in schema, (
            f"tool '{tool['name']}' inputSchema missing 'properties'"
        )


def test_mcp_tools_list_has_annotations():
    """Every built-in tool should have MCP annotations."""
    responses = _mcp_call([
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "capsem-doctor", "version": "1.0"},
            },
        },
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/list"},
    ])
    tools_resp = [r for r in responses if r.get("id") == 2]
    tools = tools_resp[0]["result"]["tools"]
    builtin_names = {"local__fetch_http", "local__grep_http", "local__http_headers"}
    for tool in tools:
        if tool["name"] in builtin_names:
            ann = tool.get("annotations")
            assert ann is not None, (
                f"builtin tool '{tool['name']}' missing annotations"
            )
            # MCP wire format uses camelCase
            assert "readOnlyHint" in ann, f"missing readOnlyHint in {tool['name']}"
            assert ann["readOnlyHint"] is True, f"{tool['name']} should be read-only"
            assert ann["destructiveHint"] is False, f"{tool['name']} should not be destructive"


def test_mcp_unknown_tool_returns_error():
    """Calling a non-existent tool should return a JSON-RPC error."""
    responses = _mcp_call([
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "capsem-doctor", "version": "1.0"},
            },
        },
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {
            "jsonrpc": "2.0",
            "id": 99,
            "method": "tools/call",
            "params": {"name": "nonexistent_tool_xyz", "arguments": {}},
        },
    ])
    call_resp = [r for r in responses if r.get("id") == 99]
    assert len(call_resp) == 1
    resp = call_resp[0]
    # Should be a JSON-RPC error (no "result" key) or isError in result
    has_error = "error" in resp or resp.get("result", {}).get("isError") is True
    assert has_error, f"unknown tool should return error: {resp}"


def test_mcp_fetch_http_missing_url():
    """fetch_http without url argument should return isError."""
    result = _init_and_call("fetch_http", {})
    assert result.get("isError") is True or "error" in str(result).lower(), (
        f"fetch_http without url should fail: {result}"
    )


def test_mcp_fetch_http_invalid_url():
    """fetch_http with a malformed URL should return isError."""
    result = _init_and_call("fetch_http", {"url": "not-a-valid-url"})
    assert result.get("isError") is True, (
        f"fetch_http with invalid URL should fail: {result}"
    )


def test_mcp_fetch_http_subpath():
    """fetch_http on the local HTML fixture returns real page content."""
    url = _require_local_mock_url("/html/about", "local MCP fetch_http subpath smoke")
    result = _init_and_call(
        "fetch_http",
        {"url": url, "max_length": 2000},
    )
    assert result.get("isError") is not True, f"fetch failed: {result}"
    text = result["content"][0]["text"]
    assert "Capsem mock server about page" in text, (
        f"fetch_http on /html/about must contain fixture text: {text[:500]}"
    )


def test_mcp_fetch_http_raw_mode():
    """fetch_http with format=raw returns HTML tags."""
    url = _require_local_mock_url("/html/about", "local MCP fetch_http raw smoke")
    result = _init_and_call(
        "fetch_http",
        {"url": url, "format": "raw", "max_length": 10000},
    )
    assert result.get("isError") is not True, f"fetch raw failed: {result}"
    text = result["content"][0]["text"]
    assert "<div" in text or "<p" in text, (
        f"raw mode must preserve HTML tags: {text[:500]}"
    )


def test_mcp_grep_http_with_pattern():
    """grep_http on the local HTML fixture finds 'Google' matches."""
    url = _require_local_mock_url("/html/about", "local MCP grep_http pattern smoke")
    result = _init_and_call(
        "grep_http",
        {"url": url, "pattern": "Google"},
    )
    assert result.get("isError") is not True, f"grep failed: {result}"
    text = result["content"][0]["text"]
    assert "Match 1" in text, (
        f"grep_http must find 'Google' on local fixture: {text[:500]}"
    )


def test_mcp_fetch_http_pagination():
    """fetch_http with small max_length shows pagination hint."""
    url = _require_local_mock_url("/html/large", "local MCP fetch_http pagination smoke")
    result = _init_and_call(
        "fetch_http",
        {"url": url, "max_length": 500},
    )
    assert result.get("isError") is not True, f"fetch failed: {result}"
    text = result["content"][0]["text"]
    assert "start_index" in text, (
        f"pagination hint must be present for large page with small max_length: {text[:500]}"
    )


def test_fastmcp_available():
    """fastmcp Python package is importable."""
    r = run("python3 -c 'import fastmcp; print(fastmcp.__version__)'")
    assert r.returncode == 0, f"fastmcp import failed: {r.stderr}"


def test_retired_snapshot_surfaces_are_absent():
    """Workspace snapshots were retired (#228): the guest can neither call the
    snapshot tools nor find the snapshot CLI."""
    responses = _mcp_call([
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "capsem-doctor", "version": "1.0"},
            },
        },
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/list"},
    ])
    tools_resp = [r for r in responses if r.get("id") == 2]
    assert len(tools_resp) == 1
    names = [t["name"] for t in tools_resp[0]["result"]["tools"]]
    assert not [name for name in names if "snapshot" in name], names
    assert run("command -v snapshots || echo absent").stdout.strip() == "absent"
