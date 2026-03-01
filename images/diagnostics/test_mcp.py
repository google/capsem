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
    assert "builtin__fetch_http" in names
    assert "builtin__grep_http" in names
    assert "builtin__http_headers" in names


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
                "name": "builtin__fetch_http",
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
                "name": "builtin__fetch_http",
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
    return call_resp[0]["result"]


# ---------------------------------------------------------------------------
# Content verification -- fetch_http must return real page text
# ---------------------------------------------------------------------------

def test_mcp_fetch_http_returns_real_content():
    """fetch_http on elie.net returns actual page content, not empty text."""
    result = _init_and_call(
        "builtin__fetch_http",
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
        "builtin__grep_http",
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
        "builtin__grep_http",
        {"url": "https://evil-blocked-domain.xyz", "pattern": "test"},
    )
    assert result["isError"] is True
    assert "blocked" in result["content"][0]["text"].lower()


def test_mcp_http_headers_blocked_domain():
    """http_headers on a blocked domain returns isError."""
    result = _init_and_call(
        "builtin__http_headers",
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
        "builtin__http_headers",
        {"url": "https://elie.net"},
    )
    assert result.get("isError") is not True, f"http_headers failed: {result}"
    text = result["content"][0]["text"]
    assert "Status:" in text, f"missing status line: {text[:300]}"
    assert "content-type" in text.lower(), f"missing content-type header: {text[:500]}"


def test_fastmcp_available():
    """fastmcp Python package is importable."""
    r = run("python3 -c 'import fastmcp; print(fastmcp.__version__)'")
    assert r.returncode == 0, f"fastmcp import failed: {r.stderr}"
