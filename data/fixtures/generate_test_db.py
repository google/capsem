#!/usr/bin/env python3
"""Generate data/fixtures/test.db -- shared test fixture for capsem-logger.

Schema matches crates/capsem-logger/src/schema.rs exactly.
Data mirrors what mock.ts currently hardcodes.

Run: python3 data/fixtures/generate_test_db.py
"""

import os
import shutil
import sqlite3
import sys

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.abspath(os.path.join(SCRIPT_DIR, "..", ".."))
DB_PATH = os.path.join(SCRIPT_DIR, "test.db")
PUBLIC_COPY = os.path.join(REPO_ROOT, "frontend", "public", "fixtures", "test.db")

# Verbatim from crates/capsem-logger/src/schema.rs (CREATE_SCHEMA)
CREATE_SCHEMA = """
    CREATE TABLE IF NOT EXISTS net_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        timestamp TEXT NOT NULL,
        domain TEXT NOT NULL,
        port INTEGER DEFAULT 443,
        decision TEXT NOT NULL,
        process_name TEXT,
        pid INTEGER,
        method TEXT,
        path TEXT,
        query TEXT,
        status_code INTEGER,
        bytes_sent INTEGER DEFAULT 0,
        bytes_received INTEGER DEFAULT 0,
        duration_ms INTEGER DEFAULT 0,
        matched_rule TEXT,
        request_headers TEXT,
        response_headers TEXT,
        request_body_preview TEXT,
        response_body_preview TEXT,
        conn_type TEXT DEFAULT 'https'
    );

    CREATE TABLE IF NOT EXISTS model_calls (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        timestamp TEXT NOT NULL,
        provider TEXT NOT NULL,
        model TEXT,
        process_name TEXT,
        pid INTEGER,
        method TEXT NOT NULL,
        path TEXT NOT NULL,
        stream INTEGER DEFAULT 0,
        system_prompt_preview TEXT,
        messages_count INTEGER DEFAULT 0,
        tools_count INTEGER DEFAULT 0,
        request_bytes INTEGER DEFAULT 0,
        request_body_preview TEXT,
        message_id TEXT,
        status_code INTEGER,
        text_content TEXT,
        thinking_content TEXT,
        stop_reason TEXT,
        input_tokens INTEGER,
        output_tokens INTEGER,
        duration_ms INTEGER DEFAULT 0,
        response_bytes INTEGER DEFAULT 0,
        estimated_cost_usd REAL DEFAULT 0,
        trace_id TEXT
    );

    CREATE TABLE IF NOT EXISTS tool_calls (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        model_call_id INTEGER NOT NULL,
        call_index INTEGER NOT NULL,
        call_id TEXT NOT NULL,
        tool_name TEXT NOT NULL,
        arguments TEXT
    );

    CREATE TABLE IF NOT EXISTS tool_responses (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        model_call_id INTEGER NOT NULL,
        call_id TEXT NOT NULL,
        content_preview TEXT,
        is_error INTEGER DEFAULT 0
    );

    CREATE INDEX IF NOT EXISTS idx_net_events_domain
        ON net_events(domain);
    CREATE INDEX IF NOT EXISTS idx_net_events_timestamp
        ON net_events(timestamp);
    CREATE INDEX IF NOT EXISTS idx_model_calls_provider_ts
        ON model_calls(provider, timestamp);
    CREATE INDEX IF NOT EXISTS idx_tool_calls_model_call
        ON tool_calls(model_call_id);
    CREATE INDEX IF NOT EXISTS idx_tool_responses_model_call
        ON tool_responses(model_call_id);
    CREATE INDEX IF NOT EXISTS idx_model_calls_trace_id
        ON model_calls(trace_id);
"""


def insert_net_event(cur, ts, domain, decision, method=None, path=None,
                     status_code=None, duration_ms=0, matched_rule=None,
                     bytes_sent=0, bytes_received=0, port=443,
                     process_name=None, pid=None, conn_type=None,
                     request_body=None, response_body=None):
    """Insert a single net_event row."""
    if conn_type is None:
        conn_type = "mitm" if decision == "allowed" else "denied"
    req_headers = "Accept: application/json\nUser-Agent: capsem-test" if method else None
    resp_headers = "Content-Type: application/json" if decision == "allowed" else None
    cur.execute(
        """INSERT INTO net_events
           (timestamp, domain, port, decision, process_name, pid,
            method, path, query, status_code,
            bytes_sent, bytes_received, duration_ms, matched_rule,
            request_headers, response_headers,
            request_body_preview, response_body_preview, conn_type)
           VALUES (?,?,?,?,?,?, ?,?,?,?, ?,?,?,?, ?,?, ?,?,?)""",
        (ts, domain, port, decision, process_name, pid,
         method, path, None, status_code,
         bytes_sent, bytes_received, duration_ms, matched_rule,
         req_headers, resp_headers,
         request_body, response_body, conn_type),
    )


def insert_model_call(cur, ts, provider, model, method, path, stream,
                      system_prompt_preview, messages_count, tools_count,
                      request_bytes, status_code, text_content,
                      thinking_content, stop_reason, input_tokens,
                      output_tokens, duration_ms, response_bytes,
                      estimated_cost_usd, trace_id, process_name="gemini",
                      pid=None, message_id=None, request_body_preview=None):
    """Insert a model_call row and return the new row ID."""
    cur.execute(
        """INSERT INTO model_calls
           (timestamp, provider, model, process_name, pid,
            method, path, stream,
            system_prompt_preview, messages_count, tools_count,
            request_bytes, request_body_preview,
            message_id, status_code, text_content, thinking_content,
            stop_reason, input_tokens, output_tokens,
            duration_ms, response_bytes, estimated_cost_usd, trace_id)
           VALUES (?,?,?,?,?, ?,?,?, ?,?,?, ?,?, ?,?,?,?, ?,?,?, ?,?,?,?)""",
        (ts, provider, model, process_name, pid,
         method, path, 1 if stream else 0,
         system_prompt_preview, messages_count, tools_count,
         request_bytes, request_body_preview,
         message_id, status_code, text_content, thinking_content,
         stop_reason, input_tokens, output_tokens,
         duration_ms, response_bytes, estimated_cost_usd, trace_id),
    )
    return cur.lastrowid


def insert_tool_call(cur, model_call_id, call_index, call_id, tool_name,
                     arguments=None):
    cur.execute(
        """INSERT INTO tool_calls
           (model_call_id, call_index, call_id, tool_name, arguments)
           VALUES (?,?,?,?,?)""",
        (model_call_id, call_index, call_id, tool_name, arguments),
    )


def insert_tool_response(cur, model_call_id, call_id, content_preview=None,
                         is_error=False):
    cur.execute(
        """INSERT INTO tool_responses
           (model_call_id, call_id, content_preview, is_error)
           VALUES (?,?,?,?)""",
        (model_call_id, call_id, content_preview, 1 if is_error else 0),
    )


SYSTEM_PROMPT = "You are a coding assistant. Help the user with their programming tasks. Use tools to read, write, and edit files. Run tests to verify your changes."


def main():
    if os.path.exists(DB_PATH):
        os.remove(DB_PATH)

    conn = sqlite3.connect(DB_PATH)
    conn.execute("PRAGMA journal_mode = WAL")
    conn.execute("PRAGMA synchronous = NORMAL")
    conn.executescript(CREATE_SCHEMA)
    cur = conn.cursor()

    # Fixed base timestamp (2026-02-27T10:00:00Z)
    BASE = "2026-02-27T10:"

    # ── Net events (27 total: 21 allowed, 6 denied) ──────────────────
    # 5-6 minutes ago
    insert_net_event(cur, f"{BASE}00:10Z", "api.github.com", "allowed",
                     "GET", "/repos/anthropics/claude-code", 200, 45,
                     "registry.github", 256, 4096,
                     process_name="git", pid=1201,
                     response_body='{"id":1,"full_name":"anthropics/claude-code","stargazers_count":12400}')
    insert_net_event(cur, f"{BASE}00:20Z", "pypi.org", "allowed",
                     "GET", "/simple/requests/", 200, 120,
                     "registry.pypi", 128, 8192,
                     process_name="pip3", pid=2301)
    insert_net_event(cur, f"{BASE}00:30Z", "api.openai.com", "denied",
                     None, None, None, 2,
                     "ai.openai (blocked)", 0, 0,
                     process_name="node", pid=3401)
    # 4-5 minutes ago
    insert_net_event(cur, f"{BASE}01:00Z", "registry.npmjs.org", "allowed",
                     "GET", "/express", 200, 200,
                     "registry.npm", 192, 12288,
                     process_name="npm", pid=2501)
    insert_net_event(cur, f"{BASE}01:10Z", "api.github.com", "allowed",
                     "GET", "/repos/google/capsem", 200, 38,
                     "registry.github", 256, 3072,
                     process_name="git", pid=1201)
    insert_net_event(cur, f"{BASE}01:20Z", "pypi.org", "allowed",
                     "GET", "/simple/flask/", 200, 95,
                     "registry.pypi", 128, 6144,
                     process_name="pip3", pid=2301)
    insert_net_event(cur, f"{BASE}01:30Z", "api.anthropic.com", "denied",
                     None, None, None, 1,
                     "ai.anthropic (blocked)", 0, 0,
                     process_name="claude", pid=3501)
    insert_net_event(cur, f"{BASE}01:40Z", "api.github.com", "allowed",
                     "PUT", "/repos/google/capsem/issues/5", 200, 82,
                     "registry.github", 420, 1024,
                     process_name="node", pid=3401,
                     request_body='{"state":"closed","labels":["bug","fixed"]}',
                     response_body='{"id":5,"state":"closed","title":"Fix auth bypass"}')
    # 3-4 minutes ago
    insert_net_event(cur, f"{BASE}02:00Z", "registry.npmjs.org", "allowed",
                     "GET", "/react", 200, 180,
                     "registry.npm", 192, 10240,
                     process_name="npm", pid=2501)
    insert_net_event(cur, f"{BASE}02:10Z", "api.github.com", "allowed",
                     "GET", "/user/repos", 200, 52,
                     "registry.github", 320, 16384,
                     process_name="git", pid=1201,
                     response_body='[{"id":1,"name":"capsem"},{"id":2,"name":"dotfiles"}]')
    insert_net_event(cur, f"{BASE}02:20Z", "pypi.org", "allowed",
                     "GET", "/simple/numpy/", 200, 110,
                     "registry.pypi", 128, 7168,
                     process_name="pip3", pid=2301)
    insert_net_event(cur, f"{BASE}02:40Z", "api.github.com", "allowed",
                     "PATCH", "/repos/google/capsem/pulls/12", 200, 95,
                     "registry.github", 380, 2048,
                     process_name="git", pid=1201,
                     request_body='{"title":"feat: add pagination","draft":false}',
                     response_body='{"id":12,"state":"open","mergeable":true}')
    # 2-3 minutes ago
    insert_net_event(cur, f"{BASE}03:00Z", "api.openai.com", "denied",
                     None, None, None, 2,
                     "ai.openai (blocked)", 0, 0,
                     process_name="node", pid=3401)
    insert_net_event(cur, f"{BASE}03:10Z", "api.github.com", "allowed",
                     "GET", "/repos/google/capsem/issues", 200, 67,
                     "registry.github", 256, 5120,
                     process_name="git", pid=1201)
    insert_net_event(cur, f"{BASE}03:20Z", "registry.npmjs.org", "allowed",
                     "GET", "/typescript", 200, 150,
                     "registry.npm", 192, 9216,
                     process_name="npm", pid=2501)
    insert_net_event(cur, f"{BASE}03:30Z", "pypi.org", "allowed",
                     "GET", "/simple/pytest/", 200, 88,
                     "registry.pypi", 128, 6400,
                     process_name="pip3", pid=2301)
    insert_net_event(cur, f"{BASE}03:40Z", "registry.npmjs.org", "denied",
                     "DELETE", "/-/package/express/dist-tags/latest", 403, 12,
                     "registry.npm.http_rule", 180, 64,
                     process_name="npm", pid=2501,
                     response_body='{"error":"DELETE method blocked by policy"}')
    # 1-2 minutes ago
    insert_net_event(cur, f"{BASE}04:00Z", "api.github.com", "allowed",
                     "GET", "/repos/anthropics/claude-code/releases", 200, 41,
                     "registry.github", 256, 3584,
                     process_name="git", pid=1201)
    insert_net_event(cur, f"{BASE}04:10Z", "api.anthropic.com", "denied",
                     None, None, None, 1,
                     "ai.anthropic (blocked)", 0, 0,
                     process_name="claude", pid=3501)
    insert_net_event(cur, f"{BASE}04:20Z", "registry.npmjs.org", "allowed",
                     "GET", "/svelte", 200, 170,
                     "registry.npm", 192, 11264,
                     process_name="npm", pid=2501)
    insert_net_event(cur, f"{BASE}04:40Z", "pypi.org", "allowed",
                     "POST", "/legacy/", 200, 280,
                     "registry.pypi", 24576, 512,
                     process_name="twine", pid=6001,
                     request_body='<multipart upload: my-package-0.1.0.tar.gz (24KB)>',
                     response_body='{"status":"ok","package":"my-package","version":"0.1.0"}')
    # last minute
    insert_net_event(cur, f"{BASE}05:00Z", "pypi.org", "allowed",
                     "GET", "/simple/rich/", 200, 102,
                     "registry.pypi", 128, 7680,
                     process_name="pip3", pid=2301)
    insert_net_event(cur, f"{BASE}05:10Z", "api.github.com", "allowed",
                     "GET", "/repos/google/capsem/pulls", 200, 55,
                     "registry.github", 256, 4608,
                     process_name="git", pid=1201)
    insert_net_event(cur, f"{BASE}05:20Z", "api.openai.com", "denied",
                     None, None, None, 2,
                     "ai.openai (blocked)", 0, 0,
                     process_name="node", pid=3401)
    insert_net_event(cur, f"{BASE}05:30Z", "generativelanguage.googleapis.com",
                     "allowed", "POST",
                     "/v1/models/gemini-pro:generateContent", 200, 320,
                     "ai.google", 512, 6400,
                     process_name="gemini", pid=4001,
                     request_body='{"contents":[{"parts":[{"text":"Explain this error"}]}],"generationConfig":{"temperature":0.7}}',
                     response_body='{"candidates":[{"content":{"parts":[{"text":"The error occurs because..."}]}}]}')
    insert_net_event(cur, f"{BASE}05:40Z", "www.google.com", "allowed",
                     "GET", "/search?q=rust+async+trait", 200, 25,
                     "search.google", 128, 2048,
                     process_name="curl", pid=5001)
    insert_net_event(cur, f"{BASE}05:50Z", "elie.net", "allowed",
                     "GET", "/", 200, 15,
                     "network.custom_allow", 64, 1024,
                     process_name="python3", pid=5101,
                     response_body='<!DOCTYPE html><html><head><title>Elie Bursztein</title></head>...')

    # ── Model calls (7 traces, 20 generations total) ──────────────────
    # Each trace uses a single model (realistic agent behavior).

    # ── Trace 1: "Fix authentication bug" (gemini-2.5-pro, 5 generations)
    tid = "trace_auth_fix"
    mc1 = insert_model_call(
        cur, f"{BASE}00:05Z", "google", "gemini-2.5-pro",
        "POST", "/v1/models/gemini-2.5-pro:generateContent", True,
        SYSTEM_PROMPT, 2, 8,
        4200, 200,
        "I'll start by reading the authentication module to understand the current implementation.",
        "The user wants to fix a bug in the auth flow. Let me look at the relevant files first.",
        "ToolUse", 1800, 420, 3200, 5600, 0.012, tid,
    )
    insert_tool_call(cur, mc1, 0, "tc_01", "read_file",
                     '{"path":"src/auth/middleware.rs"}')
    insert_tool_call(cur, mc1, 1, "tc_02", "read_file",
                     '{"path":"src/auth/session.rs"}')

    mc2 = insert_model_call(
        cur, f"{BASE}00:12Z", "google", "gemini-2.5-pro",
        "POST", "/v1/models/gemini-2.5-pro:generateContent", True,
        SYSTEM_PROMPT, 6, 8,
        6800, 200,
        "I found the issue. The session token validation skips the expiry check when the token has a refresh flag. Let me fix that.",
        None,
        "ToolUse", 3400, 680, 4100, 8200, 0.021, tid,
    )
    insert_tool_response(cur, mc2, "tc_01",
                         'pub fn validate_token(token: &str) -> Result<Claims> {\n    let claims = decode_jwt(token)?;\n    if claims.has_refresh {\n        return Ok(claims); // BUG: skips expiry check\n    }\n    if claims.exp < now() {\n        return Err(AuthError::Expired);\n    }\n    Ok(claims)\n}')
    insert_tool_response(cur, mc2, "tc_02",
                         'pub struct Session {\n    pub user_id: Uuid,\n    pub token: String,\n    pub created_at: DateTime<Utc>,\n    pub expires_at: DateTime<Utc>,\n}')
    insert_tool_call(cur, mc2, 0, "tc_03", "edit_file",
                     '{"path":"src/auth/middleware.rs","old":"if claims.has_refresh {\\n        return Ok(claims);","new":"if claims.has_refresh {\\n        if claims.exp < now() {\\n            return Err(AuthError::Expired);\\n        }\\n        return Ok(claims);"}')
    insert_tool_call(cur, mc2, 1, "tc_04", "read_file",
                     '{"path":"tests/auth_test.rs"}')

    mc3 = insert_model_call(
        cur, f"{BASE}00:20Z", "google", "gemini-2.5-pro",
        "POST", "/v1/models/gemini-2.5-pro:generateContent", True,
        SYSTEM_PROMPT, 10, 8,
        8400, 200,
        "Now I need to add a test for the refresh token expiry case and update the existing test.",
        None,
        "ToolUse", 4200, 920, 5800, 10400, 0.028, tid,
    )
    insert_tool_response(cur, mc3, "tc_03", "File edited successfully (1 change)")
    insert_tool_response(cur, mc3, "tc_04",
                         '#[test]\nfn test_valid_token() {\n    let token = create_test_token(false, future());\n    assert!(validate_token(&token).is_ok());\n}\n\n#[test]\nfn test_expired_token() {\n    let token = create_test_token(false, past());\n    assert!(validate_token(&token).is_err());\n}')
    insert_tool_call(cur, mc3, 0, "tc_05", "edit_file",
                     '{"path":"tests/auth_test.rs","old":"#[test]\\nfn test_expired_token()","new":"#[test]\\nfn test_expired_refresh_token() {\\n    let token = create_test_token(true, past());\\n    assert!(validate_token(&token).is_err());\\n}\\n\\n#[test]\\nfn test_expired_token()"}')
    insert_tool_call(cur, mc3, 1, "tc_06", "run_command",
                     '{"command":"cargo test auth"}')

    mc4 = insert_model_call(
        cur, f"{BASE}00:32Z", "google", "gemini-2.5-pro",
        "POST", "/v1/models/gemini-2.5-pro:generateContent", True,
        SYSTEM_PROMPT, 14, 8,
        9200, 200,
        "One test is failing -- the test_valid_refresh_token test needs updating since we changed the behavior. Let me fix it.",
        None,
        "ToolUse", 4800, 450, 2800, 6200, 0.018, tid,
    )
    insert_tool_response(cur, mc4, "tc_05", "File edited successfully (1 change)")
    insert_tool_response(cur, mc4, "tc_06",
                         "running 4 tests\ntest test_valid_token ... ok\ntest test_expired_token ... ok\ntest test_expired_refresh_token ... ok\ntest test_valid_refresh_token ... FAILED\n\nfailures:\n  test_valid_refresh_token: assertion failed: expected Ok, got Err(Expired)",
                         is_error=True)
    insert_tool_call(cur, mc4, 0, "tc_07", "edit_file",
                     '{"path":"tests/auth_test.rs","old":"create_test_token(true, past())","new":"create_test_token(true, future())"}')
    insert_tool_call(cur, mc4, 1, "tc_08", "run_command",
                     '{"command":"cargo test auth"}')

    mc5 = insert_model_call(
        cur, f"{BASE}00:40Z", "google", "gemini-2.5-pro",
        "POST", "/v1/models/gemini-2.5-pro:generateContent", True,
        SYSTEM_PROMPT, 18, 8,
        10600, 200,
        "All 4 tests pass now. The fix ensures that refresh tokens are also checked for expiry, which was the root cause of the authentication bypass.",
        "The fix is clean and minimal. All tests pass. The bug was that refresh tokens could bypass expiry checks entirely.",
        "EndTurn", 5200, 380, 3100, 7800, 0.016, tid,
    )
    insert_tool_response(cur, mc5, "tc_07", "File edited successfully (1 change)")
    insert_tool_response(cur, mc5, "tc_08",
                         "running 4 tests\ntest test_valid_token ... ok\ntest test_expired_token ... ok\ntest test_expired_refresh_token ... ok\ntest test_valid_refresh_token ... ok\n\ntest result: ok. 4 passed; 0 failed")

    # ── Trace 2: "Add pagination to API" (gemini-2.5-pro, 3 generations)
    tid = "trace_pagination"
    mc6 = insert_model_call(
        cur, f"{BASE}01:00Z", "google", "gemini-2.5-pro",
        "POST", "/v1/models/gemini-2.5-pro:generateContent", True,
        SYSTEM_PROMPT, 3, 6,
        3200, 200,
        "Let me look at the current API handler to understand how results are returned.",
        None,
        "ToolUse", 2100, 380, 2400, 3800, 0.009, tid,
    )
    insert_tool_call(cur, mc6, 0, "tc_09", "read_file",
                     '{"path":"src/api/handlers.rs"}')
    insert_tool_call(cur, mc6, 1, "tc_10", "read_file",
                     '{"path":"src/api/types.rs"}')

    mc7 = insert_model_call(
        cur, f"{BASE}01:08Z", "google", "gemini-2.5-pro",
        "POST", "/v1/models/gemini-2.5-pro:generateContent", True,
        SYSTEM_PROMPT, 7, 6,
        5400, 200,
        "I'll add cursor-based pagination to the list endpoint. This is more efficient than offset-based pagination for large datasets.",
        None,
        "ToolUse", 3800, 1200, 6200, 9400, 0.032, tid,
    )
    insert_tool_response(cur, mc7, "tc_09",
                         'pub async fn list_items(db: &Pool) -> Result<Json<Vec<Item>>> {\n    let items = sqlx::query_as!(Item, "SELECT * FROM items ORDER BY created_at DESC")\n        .fetch_all(db)\n        .await?;\n    Ok(Json(items))\n}')
    insert_tool_response(cur, mc7, "tc_10",
                         'pub struct Item {\n    pub id: Uuid,\n    pub name: String,\n    pub created_at: DateTime<Utc>,\n}')
    insert_tool_call(cur, mc7, 0, "tc_11", "edit_file",
                     '{"path":"src/api/handlers.rs","old":"pub async fn list_items","new":"pub async fn list_items_paginated"}')
    insert_tool_call(cur, mc7, 1, "tc_12", "write_file",
                     '{"path":"src/api/pagination.rs","content":"pub struct PaginationParams { pub cursor: Option<Uuid>, pub limit: u32 }"}')
    insert_tool_call(cur, mc7, 2, "tc_13", "run_command",
                     '{"command":"cargo test api"}')

    mc8 = insert_model_call(
        cur, f"{BASE}01:18Z", "google", "gemini-2.5-pro",
        "POST", "/v1/models/gemini-2.5-pro:generateContent", True,
        SYSTEM_PROMPT, 11, 6,
        7200, 200,
        "Pagination is implemented and all tests pass. The API now supports cursor-based pagination with configurable page size (default 20, max 100).",
        None,
        "EndTurn", 4600, 520, 3800, 6800, 0.019, tid,
    )
    insert_tool_response(cur, mc8, "tc_11", "File edited successfully (3 changes)")
    insert_tool_response(cur, mc8, "tc_12", "File written: src/api/pagination.rs (42 bytes)")
    insert_tool_response(cur, mc8, "tc_13",
                         "running 6 tests\ntest test_list_items ... ok\ntest test_pagination_first_page ... ok\ntest test_pagination_next_page ... ok\ntest test_pagination_empty ... ok\ntest test_pagination_limit ... ok\ntest test_pagination_invalid_cursor ... ok\n\ntest result: ok. 6 passed; 0 failed")

    # ── Trace 3: "Quick question" (gemini-2.0-flash, 1 generation)
    tid = "trace_quick_q"
    insert_model_call(
        cur, f"{BASE}01:30Z", "google", "gemini-2.0-flash",
        "POST", "/v1/models/gemini-2.0-flash:generateContent", False,
        None, 1, 0,
        512, 200,
        "The error `E0308: mismatched types` means the function returns `Result<(), Error>` but you're returning a bare `()`. Wrap the return value: `Ok(())`.",
        None,
        "EndTurn", 280, 85, 680, 400, 0.00004, tid,
        process_name="gemini",
    )

    # ── Trace 4: "Refactor database module" (gemini-2.5-pro, 4 generations)
    tid = "trace_db_refactor"
    mc10 = insert_model_call(
        cur, f"{BASE}02:00Z", "google", "gemini-2.5-pro",
        "POST", "/v1/models/gemini-2.5-pro:generateContent", True,
        SYSTEM_PROMPT, 4, 10,
        5600, 200,
        "Let me survey the database module structure before refactoring.",
        "This is a significant refactoring task. I should understand the full dependency graph first.",
        "ToolUse", 2800, 520, 4200, 7400, 0.015, tid,
    )
    insert_tool_call(cur, mc10, 0, "tc_14", "list_files",
                     '{"dir":"src/db/"}')
    insert_tool_call(cur, mc10, 1, "tc_15", "read_file",
                     '{"path":"src/db/mod.rs"}')
    insert_tool_call(cur, mc10, 2, "tc_16", "read_file",
                     '{"path":"src/db/queries.rs"}')
    insert_tool_call(cur, mc10, 3, "tc_17", "grep",
                     '{"pattern":"use crate::db","dir":"src/"}')

    mc11 = insert_model_call(
        cur, f"{BASE}02:10Z", "google", "gemini-2.5-pro",
        "POST", "/v1/models/gemini-2.5-pro:generateContent", True,
        SYSTEM_PROMPT, 10, 10,
        9800, 200,
        "The database module has grown too large. I'll split it into focused submodules: connection pooling, query builders, and migrations.",
        None,
        "ToolUse", 5200, 1800, 8400, 14200, 0.048, tid,
    )
    insert_tool_response(cur, mc11, "tc_14",
                         "mod.rs\nqueries.rs\nmigrations.rs\nschema.rs")
    insert_tool_response(cur, mc11, "tc_15",
                         'pub mod queries;\npub mod migrations;\npub mod schema;\n\npub struct DbPool { ... }\n\nimpl DbPool {\n    pub async fn new(url: &str) -> Result<Self> { ... }\n    pub async fn query<T>(&self, sql: &str) -> Result<Vec<T>> { ... }\n    // 200+ more lines\n}')
    insert_tool_response(cur, mc11, "tc_16",
                         'pub fn get_user_by_id(pool: &DbPool, id: Uuid) -> Result<User> { ... }\npub fn list_users(pool: &DbPool) -> Result<Vec<User>> { ... }\npub fn create_user(pool: &DbPool, user: NewUser) -> Result<User> { ... }\n// 15 more query functions')
    insert_tool_response(cur, mc11, "tc_17",
                         'src/api/handlers.rs:use crate::db::DbPool;\nsrc/api/handlers.rs:use crate::db::queries;\nsrc/auth/middleware.rs:use crate::db::DbPool;\nsrc/main.rs:use crate::db::DbPool;')
    insert_tool_call(cur, mc11, 0, "tc_18", "write_file",
                     '{"path":"src/db/pool.rs","content":"pub struct DbPool { ... }"}')
    insert_tool_call(cur, mc11, 1, "tc_19", "write_file",
                     '{"path":"src/db/builder.rs","content":"pub struct QueryBuilder { ... }"}')
    insert_tool_call(cur, mc11, 2, "tc_20", "edit_file",
                     '{"path":"src/db/mod.rs","old":"pub struct DbPool","new":"pub use pool::DbPool"}')

    mc12 = insert_model_call(
        cur, f"{BASE}02:22Z", "google", "gemini-2.5-pro",
        "POST", "/v1/models/gemini-2.5-pro:generateContent", True,
        SYSTEM_PROMPT, 16, 10,
        12400, 200,
        "Now I need to update all the import paths across the codebase and run the tests.",
        None,
        "ToolUse", 6400, 1200, 5600, 9800, 0.038, tid,
    )
    insert_tool_response(cur, mc12, "tc_18", "File written: src/db/pool.rs (184 bytes)")
    insert_tool_response(cur, mc12, "tc_19", "File written: src/db/builder.rs (256 bytes)")
    insert_tool_response(cur, mc12, "tc_20", "File edited successfully (2 changes)")
    insert_tool_call(cur, mc12, 0, "tc_21", "edit_file",
                     '{"path":"src/api/handlers.rs","old":"use crate::db::DbPool","new":"use crate::db::pool::DbPool"}')
    insert_tool_call(cur, mc12, 1, "tc_22", "edit_file",
                     '{"path":"src/auth/middleware.rs","old":"use crate::db::DbPool","new":"use crate::db::pool::DbPool"}')
    insert_tool_call(cur, mc12, 2, "tc_23", "run_command",
                     '{"command":"cargo test"}')

    mc13 = insert_model_call(
        cur, f"{BASE}02:35Z", "google", "gemini-2.5-pro",
        "POST", "/v1/models/gemini-2.5-pro:generateContent", True,
        SYSTEM_PROMPT, 20, 10,
        14200, 200,
        "Refactoring complete. The database module is now split into pool.rs (connection management), builder.rs (type-safe query construction), queries.rs (domain queries), and schema.rs (table definitions). All 24 tests pass.",
        "The refactoring is clean. Each module has a single responsibility. The public API hasn't changed, only the internal organization.",
        "EndTurn", 7200, 680, 4800, 10600, 0.028, tid,
    )
    insert_tool_response(cur, mc13, "tc_21", "File edited successfully (1 change)")
    insert_tool_response(cur, mc13, "tc_22", "File edited successfully (1 change)")
    insert_tool_response(cur, mc13, "tc_23",
                         "running 24 tests\ntest db::pool::test_connect ... ok\ntest db::pool::test_reconnect ... ok\ntest db::builder::test_select ... ok\ntest db::builder::test_where_clause ... ok\ntest db::queries::test_get_user ... ok\n... 19 more tests\n\ntest result: ok. 24 passed; 0 failed")

    # ── Trace 5: "Explain error" (gemini-2.0-flash, 2 generations)
    tid = "trace_explain_err"
    mc14 = insert_model_call(
        cur, f"{BASE}03:00Z", "google", "gemini-2.0-flash",
        "POST", "/v1/models/gemini-2.0-flash:generateContent", True,
        SYSTEM_PROMPT, 2, 4,
        1800, 200,
        "Let me look at the error context to understand what's happening.",
        None,
        "ToolUse", 900, 220, 1400, 1800, 0.0001, tid,
    )
    insert_tool_call(cur, mc14, 0, "tc_24", "read_file",
                     '{"path":"src/config.rs","line_start":88,"line_end":102}')

    mc15 = insert_model_call(
        cur, f"{BASE}03:05Z", "google", "gemini-2.0-flash",
        "POST", "/v1/models/gemini-2.0-flash:generateContent", True,
        SYSTEM_PROMPT, 4, 4,
        2800, 200,
        "The `thread 'main' panicked at 'called unwrap() on a None value'` error on line 95 happens because `config.database_url` is None when the DATABASE_URL environment variable isn't set. Replace the `.unwrap()` with `.unwrap_or_else(|| \"sqlite://data.db\".to_string())` to provide a default.",
        None,
        "EndTurn", 1400, 340, 1800, 2600, 0.0002, tid,
    )
    insert_tool_response(cur, mc15, "tc_24",
                         'pub fn load_config() -> Config {\n    let db_url = std::env::var("DATABASE_URL").ok();\n    Config {\n        database_url: db_url.unwrap(), // line 95\n        port: 8080,\n    }\n}')

    # ── Trace 6: "Write unit tests" (gemini-2.0-flash, 3 generations)
    tid = "trace_write_tests"
    mc16 = insert_model_call(
        cur, f"{BASE}03:30Z", "google", "gemini-2.0-flash",
        "POST", "/v1/models/gemini-2.0-flash:generateContent", True,
        SYSTEM_PROMPT, 3, 6,
        2400, 200,
        "I'll read the parser module to understand what needs testing.",
        None,
        "ToolUse", 1200, 280, 1600, 2200, 0.0002, tid,
    )
    insert_tool_call(cur, mc16, 0, "tc_25", "read_file",
                     '{"path":"src/parser.rs"}')

    mc17 = insert_model_call(
        cur, f"{BASE}03:38Z", "google", "gemini-2.0-flash",
        "POST", "/v1/models/gemini-2.0-flash:generateContent", True,
        SYSTEM_PROMPT, 5, 6,
        3800, 200,
        "Writing comprehensive tests for the parser module covering valid input, edge cases, and error handling.",
        None,
        "ToolUse", 2400, 1100, 4200, 6800, 0.0004, tid,
    )
    insert_tool_response(cur, mc17, "tc_25",
                         'pub fn parse_duration(s: &str) -> Result<Duration> {\n    let re = Regex::new(r"^(\\d+)(s|m|h|d)$")?;\n    let caps = re.captures(s).ok_or(ParseError::Invalid)?;\n    let n: u64 = caps[1].parse()?;\n    match &caps[2] {\n        "s" => Ok(Duration::from_secs(n)),\n        "m" => Ok(Duration::from_secs(n * 60)),\n        "h" => Ok(Duration::from_secs(n * 3600)),\n        "d" => Ok(Duration::from_secs(n * 86400)),\n        _ => unreachable!(),\n    }\n}')
    insert_tool_call(cur, mc17, 0, "tc_26", "write_file",
                     '{"path":"tests/parser_test.rs","content":"#[test] fn test_parse_seconds() { ... } #[test] fn test_parse_minutes() { ... }"}')
    insert_tool_call(cur, mc17, 1, "tc_27", "run_command",
                     '{"command":"cargo test parser"}')

    mc18 = insert_model_call(
        cur, f"{BASE}03:46Z", "google", "gemini-2.0-flash",
        "POST", "/v1/models/gemini-2.0-flash:generateContent", True,
        SYSTEM_PROMPT, 7, 6,
        4600, 200,
        "All 8 parser tests pass: valid durations (seconds, minutes, hours, days), invalid formats (empty, no unit, decimal, negative), and edge cases (zero, large values).",
        None,
        "EndTurn", 2800, 420, 2400, 4200, 0.0003, tid,
    )
    insert_tool_response(cur, mc18, "tc_26", "File written: tests/parser_test.rs (1.2KB)")
    insert_tool_response(cur, mc18, "tc_27",
                         "running 8 tests\ntest test_parse_seconds ... ok\ntest test_parse_minutes ... ok\ntest test_parse_hours ... ok\ntest test_parse_days ... ok\ntest test_parse_empty ... ok\ntest test_parse_no_unit ... ok\ntest test_parse_zero ... ok\ntest test_parse_large ... ok\n\ntest result: ok. 8 passed; 0 failed")

    # ── Trace 7: "Add CI workflow" (gemini-2.0-flash, 2 generations)
    tid = "trace_ci"
    mc19 = insert_model_call(
        cur, f"{BASE}04:10Z", "google", "gemini-2.0-flash",
        "POST", "/v1/models/gemini-2.0-flash:generateContent", True,
        SYSTEM_PROMPT, 2, 4,
        1400, 200,
        "I'll create a GitHub Actions workflow for CI with cargo test, clippy, and fmt checks.",
        None,
        "ToolUse", 800, 620, 2800, 4200, 0.0002, tid,
    )
    insert_tool_call(cur, mc19, 0, "tc_28", "write_file",
                     '{"path":".github/workflows/ci.yml","content":"name: CI\\non: [push, pull_request]\\njobs: ..."}')
    insert_tool_call(cur, mc19, 1, "tc_29", "list_files",
                     '{"dir":".github/workflows/"}')

    mc20 = insert_model_call(
        cur, f"{BASE}04:18Z", "google", "gemini-2.0-flash",
        "POST", "/v1/models/gemini-2.0-flash:generateContent", True,
        SYSTEM_PROMPT, 4, 4,
        2200, 200,
        "CI workflow created at `.github/workflows/ci.yml`. It runs on push and PR, with three jobs: test (cargo test --workspace), lint (cargo clippy -- -D warnings), and format (cargo fmt -- --check). Runs on ubuntu-latest with Rust stable.",
        None,
        "EndTurn", 1200, 380, 1600, 2400, 0.0002, tid,
    )
    insert_tool_response(cur, mc20, "tc_28", "File written: .github/workflows/ci.yml (842 bytes)")
    insert_tool_response(cur, mc20, "tc_29", "ci.yml")

    conn.commit()

    # Verify counts
    net_count = cur.execute("SELECT COUNT(*) FROM net_events").fetchone()[0]
    model_count = cur.execute("SELECT COUNT(*) FROM model_calls").fetchone()[0]
    tool_call_count = cur.execute("SELECT COUNT(*) FROM tool_calls").fetchone()[0]
    tool_resp_count = cur.execute("SELECT COUNT(*) FROM tool_responses").fetchone()[0]
    trace_count = cur.execute(
        "SELECT COUNT(DISTINCT trace_id) FROM model_calls WHERE trace_id IS NOT NULL"
    ).fetchone()[0]

    conn.close()

    # Copy to frontend/public/fixtures/ so Astro dev server serves it.
    os.makedirs(os.path.dirname(PUBLIC_COPY), exist_ok=True)
    shutil.copy2(DB_PATH, PUBLIC_COPY)

    print(f"Generated {DB_PATH}")
    print(f"Copied to {PUBLIC_COPY}")
    print(f"  net_events:     {net_count}")
    print(f"  model_calls:    {model_count}")
    print(f"  tool_calls:     {tool_call_count}")
    print(f"  tool_responses: {tool_resp_count}")
    print(f"  traces:         {trace_count}")

    return 0


if __name__ == "__main__":
    sys.exit(main())
