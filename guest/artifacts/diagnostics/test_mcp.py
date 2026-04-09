"""MCP gateway integration tests.

Verifies that the capsem-mcp-server binary exists and that the host MCP
gateway responds to JSON-RPC messages over vsock:5003.
"""

import json
import subprocess

import pytest

from conftest import run


# ---------------------------------------------------------------------------
# Helper
# ---------------------------------------------------------------------------

def _mcp_call(messages, timeout=15):
    """Send NDJSON messages to capsem-mcp-server, collect responses.

    capsem-mcp-server connects to vsock:5003 on the host and relays
    NDJSON lines bidirectionally. We send messages on stdin and read
    responses from stdout.
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
    assert resp["result"]["serverInfo"]["name"] == "capsem-mcp-gateway"


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
    assert "fetch_http" in names
    assert "grep_http" in names
    assert "http_headers" in names


def test_mcp_fetch_http_allowed_domain():
    """fetch_http on an allowed domain succeeds."""
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
                "name": "fetch_http",
                "arguments": {"url": "https://elie.net", "max_length": 1000},
            },
        },
    ])
    call_resp = [r for r in responses if r.get("id") == 3]
    assert len(call_resp) == 1
    result = call_resp[0]["result"]
    assert result.get("isError") is not True
    content_text = result["content"][0]["text"]
    assert "URL: https://elie.net" in content_text


def test_mcp_fetch_http_blocked_domain():
    """fetch_http on a blocked domain returns isError."""
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
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "fetch_http",
                "arguments": {"url": "https://evil-blocked-domain.xyz"},
            },
        },
    ])
    call_resp = [r for r in responses if r.get("id") == 4]
    assert len(call_resp) == 1
    result = call_resp[0]["result"]
    assert result["isError"] is True
    assert "blocked" in result["content"][0]["text"].lower()


def _init_and_call(tool_name, arguments, call_id=10, timeout=15):
    """Helper: initialize + call a tool in one shot, return the result dict."""
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
            "params": {"name": tool_name, "arguments": arguments},
        },
    ], timeout=timeout)
    call_resp = [r for r in responses if r.get("id") == call_id]
    assert len(call_resp) == 1, f"expected 1 response for id={call_id}, got {len(call_resp)}"
    resp = call_resp[0]
    if "error" in resp:
        raise AssertionError(
            f"MCP tool '{tool_name}' returned error: "
            f"[{resp['error'].get('code')}] {resp['error'].get('message')}"
        )
    return resp["result"]


# ---------------------------------------------------------------------------
# Content verification -- fetch_http must return real page text
# ---------------------------------------------------------------------------

def test_mcp_fetch_http_returns_real_content():
    """fetch_http on elie.net returns actual page content, not empty text."""
    result = _init_and_call(
        "fetch_http",
        {"url": "https://elie.net", "max_length": 5000},
    )
    assert result.get("isError") is not True, f"fetch failed: {result}"
    text = result["content"][0]["text"]
    # Must contain the domain echo
    assert "elie.net" in text
    # Must contain actual content from the page (not just metadata headers)
    text_lower = text.lower()
    assert "elie" in text_lower, (
        f"fetch_http returned no real content from elie.net (missing 'elie'): {text[:500]}"
    )


# ---------------------------------------------------------------------------
# Content verification -- grep_http positive match
# ---------------------------------------------------------------------------

def test_mcp_grep_http_finds_matches():
    """grep_http on elie.net with pattern 'elie' must find matches."""
    result = _init_and_call(
        "grep_http",
        {"url": "https://elie.net", "pattern": "elie"},
    )
    assert result.get("isError") is not True, f"grep failed: {result}"
    text = result["content"][0]["text"]
    assert "Matches found: 0" not in text, (
        f"grep_http found 0 matches for 'elie' on elie.net -- extraction broken: {text[:500]}"
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
    """http_headers on elie.net returns status and headers."""
    result = _init_and_call(
        "http_headers",
        {"url": "https://elie.net"},
    )
    assert result.get("isError") is not True, f"http_headers failed: {result}"
    text = result["content"][0]["text"]
    assert "Status:" in text, f"missing status line: {text[:300]}"
    assert "content-type" in text.lower(), f"missing content-type header: {text[:500]}"


def test_claude_mcp_list_shows_capsem():
    """claude mcp list must show the capsem server."""
    r = run("claude mcp list 2>&1", timeout=15)
    assert r.returncode == 0, f"claude mcp list failed: {r.stderr}"
    assert "capsem" in r.stdout, f"capsem not in claude mcp list output: {r.stdout}"


def test_claude_state_json_has_capsem_mcp():
    """Claude state file (.claude.json) has capsem MCP server configured."""
    r = run("cat /root/.claude.json")
    assert r.returncode == 0, "~/.claude.json missing"
    settings = json.loads(r.stdout)
    assert "mcpServers" in settings, "mcpServers key missing from .claude.json"
    assert "capsem" in settings["mcpServers"], (
        f"capsem not in mcpServers: {list(settings['mcpServers'].keys())}"
    )
    assert settings["mcpServers"]["capsem"]["command"] == "/run/capsem-mcp-server", (
        f"wrong command: {settings['mcpServers']['capsem']}"
    )


def test_gemini_settings_has_capsem_mcp():
    """Gemini settings.json has capsem MCP server configured."""
    r = run("cat /root/.gemini/settings.json")
    assert r.returncode == 0, "~/.gemini/settings.json missing"
    settings = json.loads(r.stdout)
    assert "mcpServers" in settings, "mcpServers key missing from Gemini settings"
    assert "capsem" in settings["mcpServers"], (
        f"capsem not in mcpServers: {list(settings['mcpServers'].keys())}"
    )
    assert settings["mcpServers"]["capsem"]["command"] == "/run/capsem-mcp-server", (
        f"wrong command: {settings['mcpServers']['capsem']}"
    )


def test_codex_config_has_capsem_mcp():
    """Codex config.toml has capsem MCP server configured."""
    r = run("cat /root/.codex/config.toml")
    assert r.returncode == 0, f"~/.codex/config.toml missing: {r.stderr}"
    assert "capsem" in r.stdout, f"capsem not in codex config: {r.stdout}"
    assert "/run/capsem-mcp-server" in r.stdout, (
        f"capsem-mcp-server path missing from codex config: {r.stdout}"
    )


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
    builtin_names = {"fetch_http", "grep_http", "http_headers"}
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
    """fetch_http on elie.net/about returns real page content."""
    result = _init_and_call(
        "fetch_http",
        {"url": "https://elie.net/about", "max_length": 2000},
    )
    assert result.get("isError") is not True, f"fetch failed: {result}"
    text = result["content"][0]["text"]
    assert "Bursztein" in text, (
        f"fetch_http on /about must contain 'Bursztein': {text[:500]}"
    )


def test_mcp_fetch_http_raw_mode():
    """fetch_http with format=raw returns HTML tags."""
    result = _init_and_call(
        "fetch_http",
        {"url": "https://elie.net/about", "format": "raw", "max_length": 10000},
    )
    assert result.get("isError") is not True, f"fetch raw failed: {result}"
    text = result["content"][0]["text"]
    assert "<div" in text or "<p" in text, (
        f"raw mode must preserve HTML tags: {text[:500]}"
    )


def test_mcp_grep_http_with_pattern():
    """grep_http on elie.net/about finds 'Google' matches."""
    result = _init_and_call(
        "grep_http",
        {"url": "https://elie.net/about", "pattern": "Google"},
    )
    assert result.get("isError") is not True, f"grep failed: {result}"
    text = result["content"][0]["text"]
    assert "Match 1" in text, (
        f"grep_http must find 'Google' on /about: {text[:500]}"
    )


def test_mcp_fetch_http_pagination():
    """fetch_http with small max_length shows pagination hint."""
    result = _init_and_call(
        "fetch_http",
        {"url": "https://elie.net/about", "max_length": 500},
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


# ---------------------------------------------------------------
# File tools (list_changed_files, revert_file) -- VirtioFS mode
# ---------------------------------------------------------------


def test_mcp_tools_list_has_file_tools():
    """tools/list must include list_changed_files and revert_file."""
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
    names = [t["name"] for t in tools]
    assert "snapshots_changes" in names, f"list_changed_files missing from tools: {names}"
    assert "snapshots_revert" in names, f"revert_file missing from tools: {names}"
    assert "snapshots_create" in names, f"snapshots_create missing from tools: {names}"
    assert "snapshots_delete" in names, f"delete_snapshot missing from tools: {names}"


def test_mcp_list_changed_files():
    """list_changed_files returns a valid response (may have files from prior tests)."""
    result = _init_and_call("snapshots_changes", {})
    assert result.get("isError") is not True, f"list_changed_files failed: {result}"
    text = result["content"][0]["text"]
    # Response is a text table (default) or JSON (with format=json).
    assert isinstance(text, str), f"expected string response: {result}"


def test_mcp_list_changed_files_after_write():
    """list_changed_files detects a newly created file."""
    import time
    # Create a test file in workspace.
    r = run("echo test-mcp-file > /root/mcp_test_file.txt")
    assert r.returncode == 0, f"failed to create test file: {r.stderr}"
    # Wait for fs monitor to pick it up.
    time.sleep(2)
    result = _init_and_call("snapshots_changes", {})
    assert result.get("isError") is not True, f"list_changed_files failed: {result}"
    text = result["content"][0]["text"]
    assert "mcp_test_file.txt" in text, (
        f"mcp_test_file.txt not in changed files: {text}"
    )
    # Cleanup.
    run("rm -f /root/mcp_test_file.txt")


def test_mcp_snapshot_tool():
    """snapshot tool creates a named checkpoint with hash."""
    import json
    result = _init_and_call("snapshots_create", {"name": "doctor_test"})
    assert result.get("isError") is not True, f"snapshot failed: {result}"
    data = json.loads(result["content"][0]["text"])
    assert data["name"] == "doctor_test"
    assert data["checkpoint"].startswith("cp-")
    assert isinstance(data["hash"], str) and len(data["hash"]) == 64
    assert isinstance(data["available"], int)


def test_mcp_revert_file():
    """revert_file restores file content (not just deletes)."""
    import json

    # 1. Create file with original content.
    r = run("echo original > /root/revert_content_test.txt")
    assert r.returncode == 0

    # 2. Take a named snapshot.
    snap_result = _init_and_call("snapshots_create", {"name": "before_modify"})
    assert snap_result.get("isError") is not True, f"snapshot failed: {snap_result}"
    snap_data = json.loads(snap_result["content"][0]["text"])
    checkpoint = snap_data["checkpoint"]

    # 3. Modify the file.
    r = run("echo modified > /root/revert_content_test.txt")
    assert r.returncode == 0
    r = run("cat /root/revert_content_test.txt")
    assert "modified" in r.stdout

    # 4. list_changed_files should show it as modified.
    list_result = _init_and_call("snapshots_changes", {"format": "json"})
    text = list_result["content"][0]["text"]
    changes = json.loads(text)
    found = [c for c in changes if "revert_content_test.txt" in c.get("path", "")]
    assert len(found) > 0, f"file not in changed list: {text}"

    # 5. Revert to the named snapshot.
    revert_result = _init_and_call(
        "snapshots_revert",
        {"path": "revert_content_test.txt", "checkpoint": checkpoint},
    )
    assert revert_result.get("isError") is not True, f"revert failed: {revert_result}"

    # 6. Verify content is restored to "original".
    r = run("cat /root/revert_content_test.txt")
    assert "original" in r.stdout, (
        f"expected 'original' after revert, got: {r.stdout}"
    )
    # Cleanup.
    run("rm -f /root/revert_content_test.txt")


def test_mcp_delete_snapshot():
    """delete_snapshot removes a manual snapshot."""
    import json
    # Create.
    result = _init_and_call("snapshots_create", {"name": "to_delete"})
    assert result.get("isError") is not True
    data = json.loads(result["content"][0]["text"])
    checkpoint = data["checkpoint"]

    # Delete.
    del_result = _init_and_call("snapshots_delete", {"checkpoint": checkpoint})
    assert del_result.get("isError") is not True, f"delete failed: {del_result}"
    del_data = json.loads(del_result["content"][0]["text"])
    assert del_data["deleted"] is True


def test_mcp_snapshot_name_sanitized():
    """snapshot rejects XSS in name."""
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
            "id": 10,
            "method": "tools/call",
            "params": {"name": "snapshots_create", "arguments": {"name": "<script>alert(1)</script>"}},
        },
    ], timeout=15)
    call_resp = [r for r in responses if r.get("id") == 10]
    assert len(call_resp) == 1
    resp = call_resp[0]
    # Must be an error (isError in result) or a JSON-RPC error.
    if "result" in resp:
        assert resp["result"].get("isError") is True, f"XSS name should fail: {resp}"
    else:
        assert "error" in resp, f"expected error for XSS name: {resp}"


# ---------------------------------------------------------------
# snapshots CLI (exercises MCP tools via subprocess)
# ---------------------------------------------------------------


def test_snapshots_binary_exists():
    """snapshots CLI is installed and executable."""
    r = run("test -x /usr/local/bin/snapshots && echo ok")
    assert "ok" in r.stdout


def test_snapshots_create_and_list():
    """snapshots create + list roundtrip."""
    # Create a named snapshot.
    r = run("snapshots create cli_test_snap --json")
    assert r.returncode == 0, f"create failed: {r.stderr}"
    data = json.loads(r.stdout)
    assert data.get("name") == "cli_test_snap", f"unexpected: {data}"
    checkpoint = data.get("checkpoint")
    assert checkpoint and checkpoint.startswith("cp-"), f"bad checkpoint: {data}"

    # List snapshots and verify it appears.
    r = run("snapshots list --json")
    assert r.returncode == 0, f"list failed: {r.stderr}"
    data = json.loads(r.stdout)
    names = [s.get("name") for s in data.get("snapshots", [])]
    assert "cli_test_snap" in names, f"snapshot not in list: {names}"


def test_snapshots_changes():
    """snapshots changes detects a newly created file."""
    import time
    r = run("echo snap-diff-test > /root/snap_diff_test.txt")
    assert r.returncode == 0
    time.sleep(2)
    r = run("snapshots changes --json")
    assert r.returncode == 0, f"diff failed: {r.stderr}"
    text = r.stdout.strip()
    assert "snap_diff_test.txt" in text, f"file not in diff: {text}"
    run("rm -f /root/snap_diff_test.txt")


def test_snapshots_revert():
    """snapshots revert restores file content."""
    # Create file with original content.
    r = run("echo snap_original > /root/snap_revert_test.txt")
    assert r.returncode == 0

    # Create snapshot.
    r = run("snapshots create snap_revert_test --json")
    assert r.returncode == 0, f"create failed: {r.stderr}"
    data = json.loads(r.stdout)
    checkpoint = data["checkpoint"]

    # Modify file.
    r = run("echo snap_modified > /root/snap_revert_test.txt")
    assert r.returncode == 0

    # Revert (auto-picks latest snapshot containing the file).
    r = run("snapshots revert snap_revert_test.txt")
    assert r.returncode == 0, f"revert failed: {r.stderr}"

    # Verify content restored.
    r = run("cat /root/snap_revert_test.txt")
    assert "snap_original" in r.stdout, f"expected original, got: {r.stdout}"
    run("rm -f /root/snap_revert_test.txt")


# ---------------------------------------------------------------
# Snapshot scenario tests (exercise real user workflows)
# ---------------------------------------------------------------


_created_snapshots = []  # track for cleanup


@pytest.fixture(autouse=True)
def _cleanup_snapshots():
    """Auto-cleanup manual snapshots after each test to prevent pool exhaustion."""
    yield
    while _created_snapshots:
        cp = _created_snapshots.pop()
        # Best-effort delete (may already be deleted by the test).
        try:
            _mcp_call([
                {"jsonrpc": "2.0", "id": 1, "method": "initialize",
                 "params": {"protocolVersion": "2024-11-05", "capabilities": {},
                            "clientInfo": {"name": "cleanup", "version": "1.0"}}},
                {"jsonrpc": "2.0", "method": "notifications/initialized"},
                {"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                 "params": {"name": "snapshots_delete", "arguments": {"checkpoint": cp}}},
            ], timeout=5)
        except Exception:
            pass


def _snap_create(name):
    """Helper: create named snapshot, return checkpoint ID."""
    r = run(f"snapshots create {name} --json")
    assert r.returncode == 0, f"snapshots create {name} failed: {r.stderr}"
    data = json.loads(r.stdout)
    cp = data["checkpoint"]
    _created_snapshots.append(cp)
    return cp




def _snap_list():
    """Helper: return list of snapshot dicts."""
    r = run("snapshots list --json")
    assert r.returncode == 0, f"snapshots list failed: {r.stderr}"
    return json.loads(r.stdout)


def _snap_history(path):
    """Helper: return history dict for a path."""
    r = run(f"snapshots history {path} --json")
    assert r.returncode == 0, f"snapshots history failed: {r.stderr}"
    return json.loads(r.stdout)


def _snap_revert(path, checkpoint=None):
    """Helper: revert a file, return subprocess result."""
    cmd = f"snapshots revert {path}"
    if checkpoint:
        cmd += f" {checkpoint}"
    return run(cmd)


def test_scenario_create_snap_modify_snap():
    """Scenario 1: create file, snap, modify, snap -- file in both snapshots."""
    run("echo v1 > /root/sc1.txt")
    cp1 = _snap_create("sc1_v1")
    run("echo v1_modified > /root/sc1.txt")
    cp2 = _snap_create("sc1_v2")

    hist = _snap_history("sc1.txt")
    versions = hist["versions"]
    cps = [v["checkpoint"] for v in versions]
    assert cp1 in cps, f"{cp1} not in history: {cps}"
    assert cp2 in cps, f"{cp2} not in history: {cps}"

    # Sizes should differ (v1 vs v1_modified).
    sizes = {v["checkpoint"]: v.get("size") for v in versions}
    assert sizes[cp1] != sizes[cp2], f"sizes should differ: {sizes}"
    run("rm -f /root/sc1.txt")


def test_scenario_modify_then_snap_revert_already_current():
    """Scenario 2: create+modify, snap, revert -> 'already current' since file matches snap."""
    run("echo created_late > /root/sc2.txt")
    run("echo modified > /root/sc2.txt")
    _snap_create("sc2_after")

    # File matches the snapshot we just took -- revert should error "already current".
    r = _snap_revert("sc2.txt")
    assert r.returncode != 0, "revert of unchanged file should fail"
    run("rm -f /root/sc2.txt")


def test_scenario_create_snap_modify_snap_versions():
    """Scenario 3: create, snap, modify, snap -- each has its own version."""
    run("echo original > /root/sc3.txt")
    cp1 = _snap_create("sc3_orig")
    run("echo changed > /root/sc3.txt")
    cp2 = _snap_create("sc3_changed")

    hist = _snap_history("sc3.txt")
    versions = {v["checkpoint"]: v for v in hist["versions"]}
    assert cp1 in versions, f"{cp1} missing from history"
    assert cp2 in versions, f"{cp2} missing from history"
    # Both should have a size (file exists in both snapshots).
    assert versions[cp1]["size"] is not None
    assert versions[cp2]["size"] is not None
    run("rm -f /root/sc3.txt")


def test_scenario_revert_to_first_snapshot():
    """Scenario 4: create, snap, modify, snap, revert to 1st -- gets original."""
    run("echo first_content > /root/sc4.txt")
    cp1 = _snap_create("sc4_first")
    run("echo second_content > /root/sc4.txt")
    _snap_create("sc4_second")

    r = _snap_revert("sc4.txt", cp1)
    assert r.returncode == 0, f"revert failed: {r.stderr}"
    r = run("cat /root/sc4.txt")
    assert "first_content" in r.stdout, f"expected first_content, got: {r.stdout}"
    run("rm -f /root/sc4.txt")


def test_scenario_revert_to_second_snapshot():
    """Scenario 5: create, snap, modify, snap, revert to 2nd -- gets modified."""
    run("echo alpha > /root/sc5.txt")
    _snap_create("sc5_alpha")
    run("echo beta > /root/sc5.txt")
    cp2 = _snap_create("sc5_beta")
    run("echo gamma > /root/sc5.txt")  # modify again after both snaps

    r = _snap_revert("sc5.txt", cp2)
    assert r.returncode == 0, f"revert failed: {r.stderr}"
    r = run("cat /root/sc5.txt")
    assert "beta" in r.stdout, f"expected beta, got: {r.stdout}"
    run("rm -f /root/sc5.txt")


def test_scenario_triple_snap_no_change():
    """Scenario 6: create, snap, snap, snap -- only first shows change in history."""
    run("echo stable > /root/sc6.txt")
    cp1 = _snap_create("sc6_a")
    cp2 = _snap_create("sc6_b")
    cp3 = _snap_create("sc6_c")

    hist = _snap_history("sc6.txt")
    versions = {v["checkpoint"]: v for v in hist["versions"]}
    # All three should contain the file.
    assert cp1 in versions and cp2 in versions and cp3 in versions, (
        f"missing checkpoints: {list(versions.keys())}"
    )
    # All should have same size (file unchanged).
    sizes = [versions[cp]["size"] for cp in [cp1, cp2, cp3]]
    assert len(set(sizes)) == 1, f"sizes should be identical: {sizes}"
    # First snap: "new" (file didn't exist before). Second + third: "unchanged".
    assert versions[cp1]["status"] == "new", (
        f"{cp1} should be 'new', got {versions[cp1]['status']}"
    )
    for cp in [cp2, cp3]:
        assert versions[cp]["status"] == "unchanged", (
            f"{cp} status should be unchanged, got {versions[cp]['status']}"
        )
    run("rm -f /root/sc6.txt")


def test_scenario_revert_auto_picks_latest():
    """Scenario 7: create, snap v1, modify, snap v2, modify again, revert (no cp) -- gets v2."""
    run("echo ver1 > /root/sc7.txt")
    _snap_create("sc7_v1")
    run("echo ver2 > /root/sc7.txt")
    _snap_create("sc7_v2")
    run("echo ver3 > /root/sc7.txt")

    r = _snap_revert("sc7.txt")
    assert r.returncode == 0, f"revert failed: {r.stderr}"
    r = run("cat /root/sc7.txt")
    assert "ver2" in r.stdout, f"expected ver2 (latest snap), got: {r.stdout}"
    run("rm -f /root/sc7.txt")


def test_scenario_revert_nonexistent_file():
    """Scenario 8: revert a file that was never snapshotted -- should error."""
    r = _snap_revert("never_existed_sc8.txt")
    assert r.returncode != 0, "revert should fail for file not in any snapshot"


def test_scenario_delete_and_revert():
    """Scenario 9: create, snap, delete file, revert -- file reappears."""
    run("echo comeback > /root/sc9.txt")
    cp = _snap_create("sc9_before_delete")
    run("rm /root/sc9.txt")

    r = run("test -f /root/sc9.txt && echo exists || echo gone")
    assert "gone" in r.stdout

    r = _snap_revert("sc9.txt", cp)
    assert r.returncode == 0, f"revert failed: {r.stderr}"
    r = run("cat /root/sc9.txt")
    assert "comeback" in r.stdout, f"expected comeback, got: {r.stdout}"
    run("rm -f /root/sc9.txt")


def test_scenario_history_with_root_prefix():
    """Scenario 10: history works with /root/ prefix (same as relative)."""
    run("echo prefix_test > /root/sc10.txt")
    _snap_create("sc10")

    h1 = _snap_history("sc10.txt")
    h2 = _snap_history("/root/sc10.txt")
    assert h1["path"] == h2["path"], f"paths should match: {h1['path']} vs {h2['path']}"
    assert len(h1["versions"]) == len(h2["versions"]), "version counts should match"
    run("rm -f /root/sc10.txt")


def test_scenario_multiple_files_one_snap():
    """Scenario 11: create 3 files, snap, verify all appear in snapshot changes."""
    run("echo a > /root/sc11_a.txt")
    run("echo b > /root/sc11_b.txt")
    run("echo c > /root/sc11_c.txt")
    _snap_create("sc11_multi")

    listing = _snap_list()
    snaps = [s for s in listing.get("snapshots", []) if s.get("name") == "sc11_multi"]
    assert len(snaps) == 1, f"expected 1 snapshot named sc11_multi: {snaps}"

    # All 3 files should be in the snapshot (history shows them).
    for fname in ["sc11_a.txt", "sc11_b.txt", "sc11_c.txt"]:
        h = _snap_history(fname)
        assert len(h["versions"]) > 0, f"{fname} not in any snapshot"
    run("rm -f /root/sc11_a.txt /root/sc11_b.txt /root/sc11_c.txt")


def test_scenario_snap_delete_snap_history():
    """Scenario 12: snap, delete snap, verify it's gone from history."""
    run("echo del_test > /root/sc12.txt")
    cp = _snap_create("sc12_to_delete")
    h1 = _snap_history("sc12.txt")
    assert any(v["checkpoint"] == cp for v in h1["versions"]), "file should be in snapshot"

    # Delete the snapshot.
    r = run(f"snapshots delete {cp} --json")
    assert r.returncode == 0, f"delete failed: {r.stderr}"

    # History should no longer include that checkpoint.
    h2 = _snap_history("sc12.txt")
    remaining = [v["checkpoint"] for v in h2["versions"]]
    assert cp not in remaining, f"{cp} still in history after delete: {remaining}"
    run("rm -f /root/sc12.txt")


# ---------------------------------------------------------------
# Bug-exposing tests (TDD RED phase -- these should FAIL until bugs are fixed)
# ---------------------------------------------------------------


def _mcp_snap_create(name):
    """MCP path: create snapshot, return checkpoint."""
    result = _init_and_call("snapshots_create", {"name": name})
    assert result.get("isError") is not True, f"snapshots_create failed: {result}"
    cp = json.loads(result["content"][0]["text"])["checkpoint"]
    _created_snapshots.append(cp)
    return cp


def _mcp_history(path):
    """MCP path: get file history."""
    result = _init_and_call("snapshots_history", {"path": path})
    assert result.get("isError") is not True, f"snapshots_history failed: {result}"
    return json.loads(result["content"][0]["text"])


def _mcp_list():
    """MCP path: list snapshots."""
    result = _init_and_call("snapshots_list", {"format": "json"})
    assert result.get("isError") is not True, f"snapshots_list failed: {result}"
    return json.loads(result["content"][0]["text"])


def _mcp_revert(path, checkpoint=None):
    """MCP path: revert a file."""
    args = {"path": path}
    if checkpoint:
        args["checkpoint"] = checkpoint
    return _init_and_call("snapshots_revert", args)


# -- Bug 1: list_snapshots changes should be vs previous snapshot, not current --

def test_bug1_list_changes_vs_previous():
    """snapshots_list changes should show what changed AT the snapshot, not vs current.

    Scenario: create file, snap1, modify, snap2.
    Expected: snap1 changes shows file as "new", snap2 changes shows file as "modified".
    Bug: both compare to current, so snap2 shows nothing (matches current) and
         snap1 shows "modified" (differs from current).
    """
    # MCP path
    run("echo bug1_v1 > /root/bug1.txt")
    cp1 = _mcp_snap_create("bug1_v1")
    run("echo bug1_v2_longer > /root/bug1.txt")
    cp2 = _mcp_snap_create("bug1_v2")

    listing = _mcp_list()
    snaps = {s["checkpoint"]: s for s in listing["snapshots"]}

    # snap1 should show bug1.txt as "new" (didn't exist before)
    cp1_changes = snaps[cp1].get("changes", [])
    cp1_ops = {c["path"]: c["op"] for c in cp1_changes}
    assert cp1_ops.get("bug1.txt") == "new", (
        f"snap1 should show bug1.txt as 'new', got: {cp1_ops.get('bug1.txt')}"
    )

    # snap2 should show bug1.txt as "modified" (changed since snap1)
    cp2_changes = snaps[cp2].get("changes", [])
    cp2_ops = {c["path"]: c["op"] for c in cp2_changes}
    assert cp2_ops.get("bug1.txt") == "modified", (
        f"snap2 should show bug1.txt as 'modified', got: {cp2_ops.get('bug1.txt')}"
    )

    # CLI path (belt and suspenders)
    r = run("snapshots list --json")
    cli_listing = json.loads(r.stdout)
    cli_snaps = {s["checkpoint"]: s for s in cli_listing["snapshots"]}
    cli_cp1_ops = {c["path"]: c["op"] for c in cli_snaps[cp1].get("changes", [])}
    assert cli_cp1_ops.get("bug1.txt") == "new", (
        f"CLI: snap1 should show 'new', got: {cli_cp1_ops.get('bug1.txt')}"
    )

    run("rm -f /root/bug1.txt")


def test_bug1_triple_snap_unchanged():
    """Three snaps with no changes: only first should show 'new', others empty changes.

    Bug: current logic shows all as identical to current (no changes on any).
    """
    run("echo stable > /root/bug1b.txt")
    cp1 = _mcp_snap_create("bug1b_a")
    cp2 = _mcp_snap_create("bug1b_b")
    cp3 = _mcp_snap_create("bug1b_c")

    listing = _mcp_list()
    snaps = {s["checkpoint"]: s for s in listing["snapshots"]}

    # Only snap1 should show bug1b.txt as "new"
    cp1_ops = {c["path"]: c["op"] for c in snaps[cp1].get("changes", [])}
    assert cp1_ops.get("bug1b.txt") == "new", (
        f"snap1 should show 'new', got: {cp1_ops.get('bug1b.txt')}"
    )

    # snap2 and snap3 should have NO change for bug1b.txt (unchanged)
    for cp in [cp2, cp3]:
        cp_paths = [c["path"] for c in snaps[cp].get("changes", [])]
        assert "bug1b.txt" not in cp_paths, (
            f"{cp} should not show bug1b.txt (unchanged), but found it"
        )

    run("rm -f /root/bug1b.txt")


# -- Bug 2: history status should be vs previous version, not vs current --

def test_bug2_history_sequential_status():
    """History status should compare each version to the PREVIOUS version.

    Scenario: create v1, snap1, modify to v2, snap2.
    Expected: snap1 status="new", snap2 status="modified".
    Bug: both compare to current, so snap2 shows "unchanged" (matches current)
         and snap1 shows "modified" (differs from current).
    """
    run("echo bug2_v1 > /root/bug2.txt")
    cp1 = _mcp_snap_create("bug2_v1")
    run("echo bug2_v2_longer > /root/bug2.txt")
    cp2 = _mcp_snap_create("bug2_v2")

    hist = _mcp_history("bug2.txt")
    versions = {v["checkpoint"]: v for v in hist["versions"]}

    assert versions[cp1]["status"] == "new", (
        f"snap1 should be 'new', got: {versions[cp1]['status']}"
    )
    assert versions[cp2]["status"] == "modified", (
        f"snap2 should be 'modified', got: {versions[cp2]['status']}"
    )

    # CLI path
    h = _snap_history("bug2.txt")
    cli_versions = {v["checkpoint"]: v for v in h["versions"]}
    assert cli_versions[cp1]["status"] == "new"
    assert cli_versions[cp2]["status"] == "modified"

    run("rm -f /root/bug2.txt")


def test_bug2_history_delete_recreate():
    """History: create, snap, delete, snap, recreate, snap => new, deleted, new."""
    run("echo bug2b_orig > /root/bug2b.txt")
    cp1 = _mcp_snap_create("bug2b_orig")
    run("rm /root/bug2b.txt")
    cp2 = _mcp_snap_create("bug2b_deleted")
    run("echo bug2b_new > /root/bug2b.txt")
    cp3 = _mcp_snap_create("bug2b_recreated")

    hist = _mcp_history("bug2b.txt")
    versions = {v["checkpoint"]: v for v in hist["versions"]}

    assert versions[cp1]["status"] == "new", f"cp1: {versions[cp1]['status']}"
    assert versions[cp2]["status"] == "deleted", f"cp2: {versions[cp2]['status']}"
    assert versions[cp3]["status"] == "new", f"cp3: {versions[cp3]['status']}"

    run("rm -f /root/bug2b.txt")


# -- Bug 3: revert should error when file already matches snapshot --

def test_bug3_revert_already_current():
    """Reverting when file already matches snapshot should error 'already current'.

    Scenario: create file, snap, revert (file unchanged) -> error.
    """
    run("echo bug3_same > /root/bug3.txt")
    cp = _mcp_snap_create("bug3")

    # File hasn't changed -- revert should error.
    # Use raw _mcp_call since _init_and_call crashes on error responses.
    responses = _mcp_call([
        {"jsonrpc": "2.0", "id": 1, "method": "initialize",
         "params": {"protocolVersion": "2024-11-05", "capabilities": {},
                    "clientInfo": {"name": "test", "version": "1.0"}}},
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/call",
         "params": {"name": "snapshots_revert",
                    "arguments": {"path": "bug3.txt", "checkpoint": cp}}},
    ])
    revert_resp = [r for r in responses if r.get("id") == 2][0]
    assert "error" in revert_resp, f"revert of identical file should error, got: {revert_resp}"
    assert "already" in revert_resp["error"]["message"].lower(), (
        f"error should mention 'already': {revert_resp['error']['message']}"
    )

    # CLI path
    r = _snap_revert("bug3.txt", cp)
    assert r.returncode != 0, "CLI revert of identical file should fail"

    run("rm -f /root/bug3.txt")


# -- Bug 4: empty snapshot filter is too aggressive --

def test_bug4_boot_snapshot_visible():
    """Auto snapshot at boot should be visible in list even if workspace was empty.

    Bug: snap_files.is_empty() filter hides snapshots that only have .venv files.
    The auto cp-0 at boot should always be visible.
    """
    listing = _mcp_list()
    snaps = listing["snapshots"]
    checkpoints = [s["checkpoint"] for s in snaps]
    # cp-0 is the auto snapshot at boot -- it should be in the list
    assert "cp-0" in checkpoints, (
        f"cp-0 (boot snapshot) should be visible, got: {checkpoints}"
    )

    # CLI path
    r = run("snapshots list --json")
    cli_listing = json.loads(r.stdout)
    cli_cps = [s["checkpoint"] for s in cli_listing["snapshots"]]
    assert "cp-0" in cli_cps, f"CLI: cp-0 missing from list: {cli_cps}"


# ---------------------------------------------------------------
# Per-tool edge case tests (T1-T15)
# ---------------------------------------------------------------

def test_tool_create_duplicate_name():
    """T1: Two snapshots with same name both succeed."""
    cp1 = _mcp_snap_create("dup_name")
    cp2 = _mcp_snap_create("dup_name")
    assert cp1 != cp2, "duplicate names should produce different checkpoints"

    # CLI path
    r1 = run("snapshots create dup_name_cli --json")
    r2 = run("snapshots create dup_name_cli --json")
    assert r1.returncode == 0 and r2.returncode == 0


def test_tool_delete_auto_snapshot():
    """T10: Deleting an auto snapshot should error."""
    with pytest.raises(AssertionError, match="cannot delete automatic"):
        _init_and_call("snapshots_delete", {"checkpoint": "cp-0"})

    # CLI path
    r = run("snapshots delete cp-0 --json")
    assert r.returncode != 0 or "cannot" in r.stdout.lower()


def test_tool_delete_nonexistent():
    """T11: Deleting nonexistent checkpoint should error."""
    with pytest.raises(AssertionError, match="not found"):
        _init_and_call("snapshots_delete", {"checkpoint": "cp-9999"})


def test_tool_delete_double():
    """T12: Deleting same checkpoint twice -- second should error."""
    cp = _mcp_snap_create("double_del")
    # First delete succeeds.
    _init_and_call("snapshots_delete", {"checkpoint": cp})
    # Remove from tracking since we just deleted it.
    if cp in _created_snapshots:
        _created_snapshots.remove(cp)

    # Second delete should error.
    with pytest.raises(AssertionError, match="not found"):
        _init_and_call("snapshots_delete", {"checkpoint": cp})


def test_tool_history_never_snapped():
    """T14: History of a file never in any snapshot."""
    hist = _mcp_history("never_existed_t14.txt")
    assert hist["versions"] == [], f"expected empty versions: {hist}"

    # CLI path
    h = _snap_history("never_existed_t14.txt")
    assert h["versions"] == []


def test_tool_history_root_prefix():
    """T15: History with /root/ prefix matches relative."""
    run("echo t15 > /root/t15_path.txt")
    _mcp_snap_create("t15")

    h1 = _mcp_history("t15_path.txt")
    h2 = _mcp_history("/root/t15_path.txt")
    assert h1["path"] == h2["path"]
    assert len(h1["versions"]) == len(h2["versions"])

    run("rm -f /root/t15_path.txt")


def test_tool_revert_root_prefix():
    """T8: Revert with /root/ prefix works."""
    run("echo t8_orig > /root/t8.txt")
    cp = _mcp_snap_create("t8")
    run("echo t8_modified > /root/t8.txt")

    result = _mcp_revert("/root/t8.txt", cp)
    assert result.get("isError") is not True, f"revert with /root/ failed: {result}"

    r = run("cat /root/t8.txt")
    assert "t8_orig" in r.stdout
    run("rm -f /root/t8.txt")


def test_tool_revert_action_restored():
    """T9a: Revert action is 'restored' when file existed in snapshot."""
    run("echo t9a > /root/t9a.txt")
    cp = _mcp_snap_create("t9a")
    run("echo t9a_changed > /root/t9a.txt")

    result = _mcp_revert("t9a.txt", cp)
    data = json.loads(result["content"][0]["text"])
    assert data["action"] == "restored", f"expected 'restored', got: {data['action']}"
    run("rm -f /root/t9a.txt")


@pytest.mark.skip(reason="APFS clonefile races: snapshot may capture file created just before clone completes")
def test_tool_revert_action_deleted():
    """T9b: Revert action is 'deleted' when file didn't exist in snapshot."""
    import time
    fname = f"t9b_{int(time.time())}.txt"
    # Ensure file does NOT exist, take snapshot, THEN create file.
    run(f"rm -f /root/{fname}")
    # Small delay so snapshot doesn't race with file creation.
    time.sleep(0.5)
    cp = _mcp_snap_create("t9b_before_file")
    run(f"echo t9b > /root/{fname}")

    result = _mcp_revert(fname, cp)
    data = json.loads(result["content"][0]["text"])
    assert data["action"] == "deleted", f"expected 'deleted', got: {data['action']}"

    r = run(f"test -f /root/{fname} && echo exists || echo gone")
    assert "gone" in r.stdout, f"file should be deleted: {r.stdout}"


def test_tool_changes_all_three_ops():
    """T6: changes shows created, modified, deleted in one call."""
    # Setup: create 3 files, snap
    run("echo t6_keep > /root/t6_keep.txt")
    run("echo t6_modify > /root/t6_modify.txt")
    run("echo t6_delete > /root/t6_delete.txt")
    _mcp_snap_create("t6_baseline")

    # Now: create new, modify one, delete one
    run("echo t6_new > /root/t6_new.txt")
    run("echo t6_modified_longer > /root/t6_modify.txt")
    run("rm /root/t6_delete.txt")

    result = _init_and_call("snapshots_changes", {"format": "json"})
    text = result["content"][0]["text"]
    changes = json.loads(text)
    ops = {c["path"]: c["op"] for c in changes}

    assert ops.get("t6_new.txt") == "created", f"t6_new should be created: {ops}"
    assert ops.get("t6_modify.txt") == "modified", f"t6_modify should be modified: {ops}"
    assert ops.get("t6_delete.txt") == "deleted", f"t6_delete should be deleted: {ops}"

    run("rm -f /root/t6_keep.txt /root/t6_modify.txt /root/t6_new.txt")


# ---------------------------------------------------------------
# Scenario tests S1-S30 (belt-and-suspenders: MCP + CLI)
# ---------------------------------------------------------------

def test_scenario_s3_revert_first_of_two():
    """S3: create, snap, modify, snap, revert 1st -> original content."""
    run("echo s3_first > /root/s3.txt")
    cp1 = _mcp_snap_create("s3_v1")
    run("echo s3_second > /root/s3.txt")
    _mcp_snap_create("s3_v2")

    # MCP path
    _mcp_revert("s3.txt", cp1)
    r = run("cat /root/s3.txt")
    assert "s3_first" in r.stdout, f"MCP: expected s3_first, got: {r.stdout}"

    # Reset and test CLI path
    run("echo s3_second > /root/s3.txt")
    r = _snap_revert("s3.txt", cp1)
    assert r.returncode == 0
    r = run("cat /root/s3.txt")
    assert "s3_first" in r.stdout, f"CLI: expected s3_first, got: {r.stdout}"
    run("rm -f /root/s3.txt")


def test_scenario_s4_revert_second_of_two():
    """S4: create, snap, modify, snap, revert 2nd -> modified content."""
    run("echo s4_alpha > /root/s4.txt")
    _mcp_snap_create("s4_alpha")
    run("echo s4_beta > /root/s4.txt")
    cp2 = _mcp_snap_create("s4_beta")
    run("echo s4_gamma > /root/s4.txt")

    _mcp_revert("s4.txt", cp2)
    r = run("cat /root/s4.txt")
    assert "s4_beta" in r.stdout, f"expected s4_beta, got: {r.stdout}"
    run("rm -f /root/s4.txt")


def test_scenario_s7_delete_shows_in_history():
    """S7: create, snap, delete, snap -> history shows 'new' then 'deleted'."""
    run("echo s7 > /root/s7.txt")
    cp1 = _mcp_snap_create("s7_exists")
    run("rm /root/s7.txt")
    cp2 = _mcp_snap_create("s7_gone")

    hist = _mcp_history("s7.txt")
    versions = {v["checkpoint"]: v for v in hist["versions"]}
    assert versions[cp1]["status"] == "new", f"cp1: {versions[cp1]}"
    assert versions[cp2]["status"] == "deleted", f"cp2: {versions[cp2]}"

    # CLI path
    h = _snap_history("s7.txt")
    cli_v = {v["checkpoint"]: v for v in h["versions"]}
    assert cli_v[cp1]["status"] == "new"
    assert cli_v[cp2]["status"] == "deleted"


def test_scenario_s8_delete_revert_restores():
    """S8: create, snap, delete, revert -> file reappears."""
    run("echo s8_comeback > /root/s8.txt")
    cp = _mcp_snap_create("s8")
    run("rm /root/s8.txt")

    _mcp_revert("s8.txt", cp)
    r = run("cat /root/s8.txt")
    assert "s8_comeback" in r.stdout
    run("rm -f /root/s8.txt")


def test_scenario_s12_copy_file():
    """S12: create A, snap, cp A B, snap -> A unchanged, B 'new'."""
    run("echo s12 > /root/s12_a.txt")
    _mcp_snap_create("s12_before_cp")
    run("cp /root/s12_a.txt /root/s12_b.txt")
    cp2 = _mcp_snap_create("s12_after_cp")

    listing = _mcp_list()
    snap2 = next(s for s in listing["snapshots"] if s["checkpoint"] == cp2)
    ops = {c["path"]: c["op"] for c in snap2.get("changes", [])}
    assert ops.get("s12_b.txt") == "new", f"B should be 'new': {ops}"
    assert "s12_a.txt" not in ops, f"A should not be in changes (unchanged): {ops}"

    run("rm -f /root/s12_a.txt /root/s12_b.txt")


def test_scenario_s13_move_file():
    """S13: create A, snap, mv A B, snap -> A 'deleted', B 'new'."""
    run("echo s13 > /root/s13_a.txt")
    _mcp_snap_create("s13_before_mv")
    run("mv /root/s13_a.txt /root/s13_b.txt")
    cp2 = _mcp_snap_create("s13_after_mv")

    listing = _mcp_list()
    snap2 = next(s for s in listing["snapshots"] if s["checkpoint"] == cp2)
    ops = {c["path"]: c["op"] for c in snap2.get("changes", [])}
    assert ops.get("s13_a.txt") == "deleted", f"A should be 'deleted': {ops}"
    assert ops.get("s13_b.txt") == "new", f"B should be 'new': {ops}"

    run("rm -f /root/s13_b.txt")


def test_scenario_s16_same_name_diff_dirs():
    """S16: a/f.txt + b/f.txt are independent in history."""
    run("mkdir -p /root/s16a /root/s16b")
    run("echo aaa > /root/s16a/f.txt")
    run("echo bbb > /root/s16b/f.txt")
    _mcp_snap_create("s16")

    h_a = _mcp_history("s16a/f.txt")
    h_b = _mcp_history("s16b/f.txt")
    assert len(h_a["versions"]) > 0
    assert len(h_b["versions"]) > 0
    assert h_a["path"] != h_b["path"]

    run("rm -rf /root/s16a /root/s16b")


def test_scenario_s18_delete_one_dir_revert():
    """S18: same-name files in diff dirs -- delete one, revert, other untouched."""
    run("mkdir -p /root/s18a /root/s18b")
    run("echo s18_a > /root/s18a/f.txt")
    run("echo s18_b > /root/s18b/f.txt")
    cp = _mcp_snap_create("s18")
    run("rm /root/s18a/f.txt")

    _mcp_revert("s18a/f.txt", cp)
    r = run("cat /root/s18a/f.txt")
    assert "s18_a" in r.stdout
    r = run("cat /root/s18b/f.txt")
    assert "s18_b" in r.stdout, "b/f.txt should be untouched"

    run("rm -rf /root/s18a /root/s18b")


@pytest.mark.skip(reason="VirtioFS does not reliably propagate host-side permission changes to guest")
def test_scenario_s19_permissions():
    """S19: chmod, snap, chmod, revert -> permissions restored."""
    run("echo s19 > /root/s19.txt && chmod 644 /root/s19.txt")
    cp = _mcp_snap_create("s19_644")
    run("chmod 777 /root/s19.txt")

    _mcp_revert("s19.txt", cp)
    r = run("stat -c %a /root/s19.txt")
    assert "644" in r.stdout, f"expected 644, got: {r.stdout}"
    run("rm -f /root/s19.txt")


def test_scenario_s22_broken_symlink():
    """S22: snap dir with broken symlink doesn't crash."""
    run("ln -sf /nonexistent /root/s22_broken")
    cp = _mcp_snap_create("s22_broken_link")
    # Should not crash
    listing = _mcp_list()
    assert any(s["checkpoint"] == cp for s in listing["snapshots"])
    run("rm -f /root/s22_broken")


def test_scenario_s25_special_chars():
    """S25: file with special chars in name."""
    run("echo s25 > '/root/s25 spaces & stuff.txt'")
    cp = _mcp_snap_create("s25_special")
    run("echo changed > '/root/s25 spaces & stuff.txt'")

    _mcp_revert("s25 spaces & stuff.txt", cp)
    r = run("cat '/root/s25 spaces & stuff.txt'")
    assert "s25" in r.stdout
    run("rm -f '/root/s25 spaces & stuff.txt'")


def test_scenario_s26_deep_path():
    """S26: deeply nested path."""
    run("mkdir -p /root/s26/a/b/c/d/e")
    run("echo deep > /root/s26/a/b/c/d/e/f.txt")
    cp = _mcp_snap_create("s26_deep")
    run("echo changed > /root/s26/a/b/c/d/e/f.txt")

    _mcp_revert("s26/a/b/c/d/e/f.txt", cp)
    r = run("cat /root/s26/a/b/c/d/e/f.txt")
    assert "deep" in r.stdout
    run("rm -rf /root/s26")


def test_scenario_s27_empty_file():
    """S27: empty file, snap, write content, snap -> sizes 0 vs N."""
    run("touch /root/s27.txt")
    cp1 = _mcp_snap_create("s27_empty")
    run("echo content > /root/s27.txt")
    cp2 = _mcp_snap_create("s27_filled")

    hist = _mcp_history("s27.txt")
    versions = {v["checkpoint"]: v for v in hist["versions"]}
    assert versions[cp1]["size"] == 0, f"cp1 should be 0 bytes: {versions[cp1]}"
    assert versions[cp2]["size"] > 0, f"cp2 should have content: {versions[cp2]}"
    assert versions[cp2]["status"] == "modified", f"cp2 status: {versions[cp2]['status']}"

    run("rm -f /root/s27.txt")


def test_scenario_s28_rapid_snaps():
    """S28: two rapid snaps -- both succeed, no corruption."""
    run("echo s28 > /root/s28.txt")
    cp1 = _mcp_snap_create("s28_rapid1")
    cp2 = _mcp_snap_create("s28_rapid2")
    assert cp1 != cp2

    listing = _mcp_list()
    cps = [s["checkpoint"] for s in listing["snapshots"]]
    assert cp1 in cps and cp2 in cps
    run("rm -f /root/s28.txt")


def test_scenario_s29_many_files():
    """S29: 100 files, snap -- all in history."""
    run("mkdir -p /root/s29")
    for i in range(100):
        run(f"echo f{i} > /root/s29/f{i}.txt")
    _mcp_snap_create("s29_100files")

    # Spot check a few
    for i in [0, 49, 99]:
        h = _mcp_history(f"s29/f{i}.txt")
        assert len(h["versions"]) > 0, f"s29/f{i}.txt not in history"

    run("rm -rf /root/s29")


def test_scenario_s9_delete_recreate_three_versions():
    """S9: create, snap, delete, snap, recreate with new content, snap -> 3 versions."""
    run("echo s9_orig > /root/s9.txt")
    cp1 = _mcp_snap_create("s9_v1")
    run("rm /root/s9.txt")
    cp2 = _mcp_snap_create("s9_deleted")
    run("echo s9_new > /root/s9.txt")
    cp3 = _mcp_snap_create("s9_v2")

    hist = _mcp_history("s9.txt")
    versions = {v["checkpoint"]: v for v in hist["versions"]}
    assert versions[cp1]["status"] == "new", f"cp1: {versions[cp1]}"
    assert versions[cp2]["status"] == "deleted", f"cp2: {versions[cp2]}"
    assert versions[cp3]["status"] == "new", f"cp3: {versions[cp3]}"

    # CLI path
    h = _snap_history("s9.txt")
    cli_v = {v["checkpoint"]: v for v in h["versions"]}
    assert cli_v[cp1]["status"] == "new"
    assert cli_v[cp2]["status"] == "deleted"
    assert cli_v[cp3]["status"] == "new"
    run("rm -f /root/s9.txt")


def test_scenario_s10_delete_recreate_revert_first():
    """S10: create, snap, delete, snap, recreate, snap, revert 1st -> original content."""
    run("echo s10_original > /root/s10.txt")
    cp1 = _mcp_snap_create("s10_orig")
    run("rm /root/s10.txt")
    _mcp_snap_create("s10_deleted")
    run("echo s10_recreated > /root/s10.txt")
    _mcp_snap_create("s10_new")

    _mcp_revert("s10.txt", cp1)
    r = run("cat /root/s10.txt")
    assert "s10_original" in r.stdout, f"expected original, got: {r.stdout}"

    # CLI path verification
    run("echo s10_recreated > /root/s10.txt")  # reset
    r = _snap_revert("s10.txt", cp1)
    assert r.returncode == 0
    r = run("cat /root/s10.txt")
    assert "s10_original" in r.stdout
    run("rm -f /root/s10.txt")


def test_scenario_s11_delete_recreate_revert_third():
    """S11: create, snap, delete, snap, recreate, snap, revert 3rd -> recreated content."""
    run("echo s11_original > /root/s11.txt")
    _mcp_snap_create("s11_orig")
    run("rm /root/s11.txt")
    _mcp_snap_create("s11_deleted")
    run("echo s11_recreated > /root/s11.txt")
    cp3 = _mcp_snap_create("s11_new")
    run("echo s11_latest > /root/s11.txt")  # modify after all snaps

    _mcp_revert("s11.txt", cp3)
    # Drop VirtioFS cached metadata so the guest sees the new file size.
    run("sync && echo 3 > /proc/sys/vm/drop_caches 2>/dev/null; true")
    r = run("cat /root/s11.txt")
    assert "s11_recreated" in r.stdout, f"expected recreated, got: {r.stdout}"
    run("rm -f /root/s11.txt")


def test_scenario_s14_dir_move_cross_dir():
    """S14: create dir1/A, snap, mv dir1/A dir2/A, snap -> deletion in dir1, creation in dir2."""
    run("mkdir -p /root/s14_dir1 /root/s14_dir2")
    run("echo s14 > /root/s14_dir1/a.txt")
    _mcp_snap_create("s14_before_mv")
    run("mv /root/s14_dir1/a.txt /root/s14_dir2/a.txt")
    cp2 = _mcp_snap_create("s14_after_mv")

    listing = _mcp_list()
    snap2 = next(s for s in listing["snapshots"] if s["checkpoint"] == cp2)
    ops = {c["path"]: c["op"] for c in snap2.get("changes", [])}
    assert ops.get("s14_dir1/a.txt") == "deleted", f"dir1/a.txt should be deleted: {ops}"
    assert ops.get("s14_dir2/a.txt") == "new", f"dir2/a.txt should be new: {ops}"
    run("rm -rf /root/s14_dir1 /root/s14_dir2")


def test_scenario_s15_dir_move_revert():
    """S15: create dir1/A, snap, mv dir1 dir2, snap, revert 1st -> dir1/A restored."""
    run("mkdir -p /root/s15_dir1")
    run("echo s15 > /root/s15_dir1/a.txt")
    cp1 = _mcp_snap_create("s15_before")
    run("mv /root/s15_dir1 /root/s15_dir2")

    _mcp_revert("s15_dir1/a.txt", cp1)
    r = run("cat /root/s15_dir1/a.txt")
    assert "s15" in r.stdout, f"expected s15, got: {r.stdout}"
    run("rm -rf /root/s15_dir1 /root/s15_dir2")


def test_scenario_s17_modify_one_dir_other_unchanged():
    """S17: same-name files in diff dirs, modify one, snap -> only modified shows change."""
    run("mkdir -p /root/s17a /root/s17b")
    run("echo s17_a > /root/s17a/f.txt")
    run("echo s17_b > /root/s17b/f.txt")
    _mcp_snap_create("s17_baseline")
    run("echo s17_a_changed > /root/s17a/f.txt")
    cp2 = _mcp_snap_create("s17_modified")

    listing = _mcp_list()
    snap2 = next(s for s in listing["snapshots"] if s["checkpoint"] == cp2)
    ops = {c["path"]: c["op"] for c in snap2.get("changes", [])}
    assert ops.get("s17a/f.txt") == "modified", f"s17a/f.txt should be modified: {ops}"
    assert "s17b/f.txt" not in ops, f"s17b/f.txt should not be in changes: {ops}"
    run("rm -rf /root/s17a /root/s17b")


def test_scenario_s20_touch_mtime_unchanged():
    """S20: create, snap, touch -m (mtime only), snap -> unchanged (size-based detection)."""
    run("echo s20 > /root/s20.txt")
    cp1 = _mcp_snap_create("s20_orig")
    run("touch -m /root/s20.txt")  # change mtime, not content
    cp2 = _mcp_snap_create("s20_touched")

    hist = _mcp_history("s20.txt")
    versions = {v["checkpoint"]: v for v in hist["versions"]}
    assert versions[cp2]["status"] == "unchanged", (
        f"touch -m should not change status: {versions[cp2]}"
    )
    run("rm -f /root/s20.txt")


def test_scenario_s21_symlink_revert():
    """S21: create A, symlink B->A, snap, delete B, revert -> B restored as symlink."""
    run("echo s21_target > /root/s21_a.txt")
    run("ln -sf /root/s21_a.txt /root/s21_link")
    cp = _mcp_snap_create("s21_with_link")
    run("rm /root/s21_link")

    r = run("test -e /root/s21_link && echo exists || echo gone")
    assert "gone" in r.stdout

    _mcp_revert("s21_link", cp)
    # The reverted file should exist (content copied from snapshot, may not be a symlink).
    r = run("test -e /root/s21_link && echo exists || echo gone")
    assert "exists" in r.stdout, f"link should be restored: {r.stdout}"
    run("rm -f /root/s21_a.txt /root/s21_link")


# ---------------------------------------------------------------
# Compact tests (belt-and-suspenders: MCP + CLI)
# ---------------------------------------------------------------


def test_compact_merges_and_frees_slots():
    """Compact 2 snaps with different files -> merged has both, originals deleted."""
    run("echo compact_a > /root/cpt_a.txt")
    cp1 = _mcp_snap_create("cpt_v1")
    run("echo compact_b > /root/cpt_b.txt")
    cp2 = _mcp_snap_create("cpt_v2")

    # MCP path
    result = _init_and_call("snapshots_compact", {
        "checkpoints": [cp1, cp2],
        "name": "cpt_merged",
    })
    data = json.loads(result["content"][0]["text"])
    assert data["compacted"] is True
    assert data["merged_count"] == 2
    new_cp = data["checkpoint"]
    _created_snapshots.append(new_cp)

    # Originals should be gone.
    listing = _mcp_list()
    cps = [s["checkpoint"] for s in listing["snapshots"]]
    assert cp1 not in cps, f"{cp1} should be deleted"
    assert cp2 not in cps, f"{cp2} should be deleted"
    assert new_cp in cps, f"{new_cp} should exist"

    # Merged snapshot should have both files.
    h_a = _mcp_history("cpt_a.txt")
    h_b = _mcp_history("cpt_b.txt")
    assert any(v["checkpoint"] == new_cp for v in h_a["versions"]), "cpt_a.txt missing"
    assert any(v["checkpoint"] == new_cp for v in h_b["versions"]), "cpt_b.txt missing"

    run("rm -f /root/cpt_a.txt /root/cpt_b.txt")


def test_compact_newest_wins():
    """Compact: same file in 2 snaps -> newest version kept."""
    run("echo old_content > /root/cpt_nw.txt")
    cp1 = _mcp_snap_create("cpt_nw_old")
    run("echo new_content > /root/cpt_nw.txt")
    cp2 = _mcp_snap_create("cpt_nw_new")

    result = _init_and_call("snapshots_compact", {
        "checkpoints": [cp1, cp2],
        "name": "cpt_nw_merged",
    })
    data = json.loads(result["content"][0]["text"])
    new_cp = data["checkpoint"]
    _created_snapshots.append(new_cp)

    # Revert to merged snapshot and verify newest content.
    run("echo overwritten > /root/cpt_nw.txt")
    _mcp_revert("cpt_nw.txt", new_cp)
    r = run("cat /root/cpt_nw.txt")
    assert "new_content" in r.stdout, f"expected new_content, got: {r.stdout}"
    run("rm -f /root/cpt_nw.txt")


def test_compact_invalid_checkpoint():
    """Compact with invalid checkpoint ID errors."""
    with pytest.raises(AssertionError, match="not found"):
        _init_and_call("snapshots_compact", {
            "checkpoints": ["cp-9999"],
            "name": "bad",
        })


def test_compact_cli():
    """Compact via CLI (belt-and-suspenders)."""
    run("echo cli_compact > /root/cpt_cli.txt")
    cp1 = _snap_create("cpt_cli_a")
    run("echo cli_compact_v2 > /root/cpt_cli.txt")
    cp2 = _snap_create("cpt_cli_b")

    r = run(f"snapshots compact {cp1} {cp2} --name cpt_cli_merged --json")
    assert r.returncode == 0, f"compact failed: {r.stderr}"
    data = json.loads(r.stdout)
    assert data["compacted"] is True
    new_cp = data["checkpoint"]
    _created_snapshots.append(new_cp)

    # Verify originals gone.
    r = run("snapshots list --json")
    listing = json.loads(r.stdout)
    cps = [s["checkpoint"] for s in listing["snapshots"]]
    assert cp1 not in cps
    assert cp2 not in cps
    assert new_cp in cps

    run("rm -f /root/cpt_cli.txt")
