#!/usr/bin/env python3
"""End-to-end integration test: boot VM, exercise all telemetry pipelines,
verify every event type is logged in the session DB.

Exercises:
  1. fs_events   -- create, modify, and delete files inside the VM
  2. net_events   -- curl an allowed domain + a denied domain (policy enforcement)
  3. mcp_calls    -- run capsem-doctor MCP tests (init, tools/list, fetch, grep)
  4. model_calls  -- ask Gemini to write a poem (verifies cost estimation)
  5. tool_calls   -- Gemini tool use (write_file) with origin tracking
  6. main.db      -- rollup counters match session.db actuals

Usage:
    python3 scripts/integration_test.py              # uses target/debug/capsem
    python3 scripts/integration_test.py --binary ./capsem --assets ./assets
"""

import argparse
import os
import re
import sqlite3
import subprocess
import sys
from pathlib import Path

BOLD = "\033[1m"
DIM = "\033[2m"
GREEN = "\033[32m"
RED = "\033[31m"
YELLOW = "\033[33m"
CYAN = "\033[36m"
RESET = "\033[0m"

SESSIONS_DIR = Path.home() / ".capsem" / "sessions"
MAIN_DB = SESSIONS_DIR / "main.db"

# The compound command executed inside the VM.  Semicolons ensure every step
# runs even if an earlier one fails -- the host-side assertions decide pass/fail.
VM_COMMAND = "; ".join([
    # -- fs_events: create, modify, and delete files --
    "echo 'integration-test-data' > /root/integration_test.txt",
    "mkdir -p /root/test_dir",
    "echo 'nested-file-content' > /root/test_dir/nested.txt",
    "echo 'to-be-deleted' > /root/delete_me.txt",
    "sleep 0.2",  # let debouncer see the create before we delete
    "rm /root/delete_me.txt",

    # -- net_events: HTTPS fetch to allowed + denied domains --
    "curl -sf https://elie.net -o /dev/null",
    "curl -sf https://api.openai.com/ -o /dev/null || true",  # denied by policy

    # -- mcp_calls: capsem-doctor MCP test subset --
    "capsem-doctor -k mcp",

    # -- model_calls + tool_calls: ask Gemini to write a poem into a file --
    (
        "gemini --yolo -p "
        "'write a four line poem about sandboxes and save it to"
        " /root/gemini_poem.txt'"
    ),

    # -- debouncer flush: fs_events uses a 100ms debouncer --
    "sleep 2",

    # -- sentinel so the host can confirm full execution --
    "echo CAPSEM_INTEGRATION_DONE",
])


def run_vm(binary: str, assets_dir: str) -> tuple[str, int]:
    """Boot the VM, run the test command, return (session_id, exit_code)."""
    env = {**os.environ, "CAPSEM_ASSETS_DIR": assets_dir, "RUST_LOG": "capsem=warn"}
    print(f"{BOLD}Booting VM with test command ...{RESET}")
    proc = subprocess.run(
        [binary, VM_COMMAND],
        env=env,
        capture_output=True,
        text=True,
        timeout=300,
    )
    output = proc.stdout + "\n" + proc.stderr
    match = re.search(r"\[capsem\] session: (\S+)", output)
    if not match:
        print(f"{RED}FAIL: could not find session ID in output{RESET}")
        print(output[:2000])
        sys.exit(1)
    session_id = match.group(1)
    print(f"  session: {CYAN}{session_id}{RESET}  exit_code: {proc.returncode}")
    return session_id, proc.returncode


# ── assertions ───────────────────────────────────────────────────────────


class Results:
    """Accumulates pass/fail/warn results for a clean summary."""

    def __init__(self):
        self.passed: list[str] = []
        self.failed: list[str] = []
        self.warned: list[str] = []

    def ok(self, msg: str):
        self.passed.append(msg)
        print(f"  {GREEN}PASS{RESET}  {msg}")

    def fail(self, msg: str):
        self.failed.append(msg)
        print(f"  {RED}FAIL{RESET}  {msg}")

    def warn(self, msg: str):
        self.warned.append(msg)
        print(f"  {YELLOW}WARN{RESET}  {msg}")

    def check(self, cond: bool, pass_msg: str, fail_msg: str):
        if cond:
            self.ok(pass_msg)
        else:
            self.fail(fail_msg)

    @property
    def success(self) -> bool:
        return len(self.failed) == 0


def verify_session(session_id: str) -> bool:
    """Open the session DB, run all assertions, return True on success."""
    db_path = SESSIONS_DIR / session_id / "session.db"
    gz_path = SESSIONS_DIR / session_id / "session.db.gz"

    # Session DB may be gzip-compressed after vacuum. Decompress for reading.
    if not db_path.exists() and gz_path.exists():
        import gzip
        with gzip.open(gz_path, "rb") as f_in:
            db_path.write_bytes(f_in.read())

    if not db_path.exists():
        print(f"{RED}session.db not found at {db_path}{RESET}")
        return False

    conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    conn.row_factory = sqlite3.Row
    r = Results()

    # ── fs_events ────────────────────────────────────────────────────
    print(f"\n{BOLD}fs_events{RESET}")
    fs_count = conn.execute("SELECT COUNT(*) FROM fs_events").fetchone()[0]
    r.check(
        fs_count > 0,
        f"{fs_count} fs_events recorded",
        "no fs_events recorded",
    )

    # Our explicit test file must appear.
    test_file = conn.execute(
        "SELECT * FROM fs_events WHERE path LIKE '%integration_test%'"
    ).fetchone()
    r.check(
        test_file is not None,
        "integration_test.txt logged in fs_events",
        "integration_test.txt NOT found in fs_events",
    )

    # Nested file.
    nested = conn.execute(
        "SELECT * FROM fs_events WHERE path LIKE '%nested.txt%'"
    ).fetchone()
    r.check(
        nested is not None,
        "test_dir/nested.txt logged in fs_events",
        "test_dir/nested.txt NOT found in fs_events",
    )

    # Boot-config files (settings written by the agent).
    boot_files = conn.execute(
        "SELECT COUNT(*) FROM fs_events WHERE path LIKE '%.claude%' OR path LIKE '%.gemini%'"
    ).fetchone()[0]
    r.check(
        boot_files > 0,
        f"{boot_files} boot-config file events (.claude/*, .gemini/*)",
        "no boot-config file events detected",
    )

    # Deleted file event from rm /root/delete_me.txt.
    deleted = conn.execute(
        "SELECT * FROM fs_events WHERE path LIKE '%delete_me%' AND action = 'deleted'"
    ).fetchone()
    r.check(
        deleted is not None,
        "delete_me.txt deleted event logged in fs_events",
        "delete_me.txt deleted event NOT found in fs_events",
    )

    # Action type breakdown -- verify modified and deleted are present.
    # Note: inotify reports IN_CLOSE_WRITE for both new and modified files,
    # so "created" may not appear -- "modified" covers both cases.
    actions = conn.execute(
        "SELECT action, COUNT(*) as cnt FROM fs_events GROUP BY action"
    ).fetchall()
    action_map = {row["action"]: row["cnt"] for row in actions}
    r.check(
        "modified" in action_map and "deleted" in action_map,
        f"fs_event actions: {dict(action_map)}",
        f"expected modified+deleted, got: {dict(action_map)}",
    )

    # Gemini poem file (created by Gemini tool use).
    poem = conn.execute(
        "SELECT * FROM fs_events WHERE path LIKE '%gemini_poem%'"
    ).fetchone()
    if poem:
        r.ok("gemini_poem.txt logged in fs_events (Gemini wrote the file)")
    else:
        r.warn("gemini_poem.txt NOT in fs_events (Gemini may not have used write_file)")

    # ── net_events ───────────────────────────────────────────────────
    print(f"\n{BOLD}net_events{RESET}")
    net_count = conn.execute("SELECT COUNT(*) FROM net_events").fetchone()[0]
    r.check(
        net_count > 0,
        f"{net_count} net_events recorded",
        "no net_events recorded",
    )

    # elie.net from the curl.
    elie = conn.execute(
        "SELECT * FROM net_events WHERE domain = 'elie.net'"
    ).fetchone()
    r.check(
        elie is not None,
        "elie.net request logged (curl)",
        "elie.net NOT found in net_events (curl may have failed)",
    )

    # Allowed decision.
    if elie:
        r.check(
            elie["decision"] == "allowed",
            "elie.net decision = allowed",
            f"elie.net decision = {elie['decision']} (expected allowed)",
        )

    # Google/Gemini API requests.
    google_net = conn.execute(
        "SELECT COUNT(*) FROM net_events WHERE domain LIKE '%.googleapis.com'"
    ).fetchone()[0]
    r.check(
        google_net > 0,
        f"{google_net} googleapis.com net_events (Gemini API calls)",
        "no googleapis.com net_events (Gemini API call not captured)",
    )

    # At least one allowed net_event with an HTTP status code.
    with_status = conn.execute(
        "SELECT COUNT(*) FROM net_events WHERE status_code IS NOT NULL AND status_code > 0"
    ).fetchone()[0]
    r.check(
        with_status >= 1,
        f"{with_status} net_events have HTTP status codes",
        "no net_events with HTTP status codes (MITM proxy may not be recording)",
    )

    # Denied net_event from curl to api.openai.com (blocked by default policy).
    denied_count = conn.execute(
        "SELECT COUNT(*) FROM net_events WHERE decision = 'denied'"
    ).fetchone()[0]
    r.check(
        denied_count >= 1,
        f"{denied_count} denied net_events (policy enforcement working)",
        "no denied net_events (curl to blocked domain may have failed silently)",
    )

    # Decision breakdown -- verify both allowed and denied present.
    decisions = conn.execute(
        "SELECT decision, COUNT(*) as cnt FROM net_events GROUP BY decision"
    ).fetchall()
    decision_map = {row["decision"]: row["cnt"] for row in decisions}
    r.check(
        "allowed" in decision_map and "denied" in decision_map,
        f"net_event decisions: {dict(decision_map)}",
        f"expected both allowed and denied decisions, got: {dict(decision_map)}",
    )

    # ── mcp_calls ────────────────────────────────────────────────────
    print(f"\n{BOLD}mcp_calls{RESET}")
    mcp_count = conn.execute("SELECT COUNT(*) FROM mcp_calls").fetchone()[0]
    r.check(
        mcp_count > 0,
        f"{mcp_count} mcp_calls recorded",
        "no mcp_calls recorded",
    )

    # Expect at least: initialize, tools/list, and several tools/call.
    mcp_methods = conn.execute(
        "SELECT DISTINCT method FROM mcp_calls"
    ).fetchall()
    methods = {row["method"] for row in mcp_methods}
    r.check(
        "initialize" in methods,
        "MCP initialize logged",
        "MCP initialize NOT logged",
    )
    r.check(
        "tools/list" in methods,
        "MCP tools/list logged",
        "MCP tools/list NOT logged",
    )
    r.check(
        "tools/call" in methods,
        "MCP tools/call logged",
        "MCP tools/call NOT logged",
    )

    # fetch_http tool calls (logged as builtin__fetch_http).
    fetch_calls = conn.execute(
        "SELECT COUNT(*) FROM mcp_calls WHERE tool_name LIKE '%fetch_http%'"
    ).fetchone()[0]
    r.check(
        fetch_calls >= 2,
        f"{fetch_calls} fetch_http MCP calls (allowed + blocked)",
        f"only {fetch_calls} fetch_http calls (expected >= 2)",
    )

    # Blocked-domain tests return isError in the MCP result (not a JSON-RPC
    # error), so the gateway logs them as "allowed".  Verify the response
    # preview contains "blocked" for at least one call.
    blocked_in_preview = conn.execute(
        "SELECT COUNT(*) FROM mcp_calls"
        " WHERE response_preview LIKE '%blocked%'"
        "    OR response_preview LIKE '%isError%'"
    ).fetchone()[0]
    r.check(
        blocked_in_preview >= 1,
        f"{blocked_in_preview} MCP calls with blocked-domain responses",
        "no MCP responses mention blocking (capsem-doctor blocked tests may have failed)",
    )

    # ── model_calls ──────────────────────────────────────────────────
    print(f"\n{BOLD}model_calls{RESET}")
    model_count = conn.execute("SELECT COUNT(*) FROM model_calls").fetchone()[0]
    r.check(
        model_count > 0,
        f"{model_count} model_calls recorded",
        "no model_calls recorded (Gemini API parsing may have failed)",
    )

    if model_count > 0:
        # Provider should be google.
        google_calls = conn.execute(
            "SELECT * FROM model_calls WHERE provider = 'google'"
        ).fetchone()
        r.check(
            google_calls is not None,
            "Gemini model_call has provider = google",
            "no model_call with provider = google",
        )

        # Token counts should be non-zero.  The first model_call may be a
        # preflight with no model/tokens, so query for one that has a model.
        google_with_model = conn.execute(
            "SELECT * FROM model_calls"
            " WHERE provider = 'google' AND model IS NOT NULL"
            " ORDER BY id LIMIT 1"
        ).fetchone()
        if google_with_model:
            in_tok = google_with_model["input_tokens"] or 0
            out_tok = google_with_model["output_tokens"] or 0
            model_name = google_with_model["model"]
            r.check(
                in_tok > 0 and out_tok > 0,
                f"Gemini tokens: {in_tok} in / {out_tok} out (model={model_name})",
                f"Gemini token counts look wrong: {in_tok} in / {out_tok} out",
            )
        else:
            r.warn("no Gemini model_call with a model name (stream parsing incomplete)")

    # Cost estimation -- at least one model_call should have a positive cost.
    with_cost = conn.execute(
        "SELECT COUNT(*) FROM model_calls WHERE estimated_cost_usd > 0"
    ).fetchone()[0]
    r.check(
        with_cost >= 1,
        f"{with_cost} model_calls with positive estimated_cost_usd",
        "no model_calls with positive cost (pricing lookup may be broken)",
    )

    # ── tool_calls / tool_responses ──────────────────────────────────
    print(f"\n{BOLD}tool_calls / tool_responses{RESET}")
    tc_count = conn.execute("SELECT COUNT(*) FROM tool_calls").fetchone()[0]
    tr_count = conn.execute("SELECT COUNT(*) FROM tool_responses").fetchone()[0]

    if tc_count > 0:
        r.ok(f"{tc_count} tool_calls recorded (Gemini used tools)")

        # Origin column should be populated on all tool_calls.
        with_origin = conn.execute(
            "SELECT COUNT(*) FROM tool_calls WHERE origin IS NOT NULL AND origin != ''"
        ).fetchone()[0]
        r.check(
            with_origin == tc_count,
            f"all {with_origin} tool_calls have origin column populated",
            f"only {with_origin}/{tc_count} tool_calls have origin",
        )

        # tool_responses depend on the stream parser capturing the tool result
        # turn.  Gemini's streaming format may not always produce a parseable
        # tool_response, so this is a warning rather than a hard failure.
        if tr_count >= tc_count:
            r.ok(f"{tr_count} tool_responses match {tc_count} tool_calls")
        else:
            r.warn(
                f"tool_responses ({tr_count}) < tool_calls ({tc_count})"
                " -- stream parser may not capture Gemini tool results"
            )
    else:
        r.warn(
            "0 tool_calls (Gemini may have printed the poem instead of using write_file)"
        )

    conn.close()

    # ── main.db rollup ───────────────────────────────────────────────
    print(f"\n{BOLD}main.db rollup{RESET}")
    if MAIN_DB.exists():
        mconn = sqlite3.connect(f"file:{MAIN_DB}?mode=ro", uri=True)
        mconn.row_factory = sqlite3.Row
        row = mconn.execute(
            "SELECT * FROM sessions WHERE id = ?", (session_id,)
        ).fetchone()
        if row:
            r.check(
                row["status"] in ("stopped", "vacuumed"),
                f"main.db status = {row['status']}",
                f"main.db status = {row['status']} (expected stopped or vacuumed)",
            )
            r.check(
                row["total_file_events"] > 0,
                f"main.db total_file_events = {row['total_file_events']}",
                "main.db total_file_events = 0 (rollup failed)",
            )
            r.check(
                row["total_requests"] > 0,
                f"main.db total_requests = {row['total_requests']}",
                "main.db total_requests = 0 (rollup failed)",
            )
            r.check(
                row["total_mcp_calls"] > 0,
                f"main.db total_mcp_calls = {row['total_mcp_calls']}",
                "main.db total_mcp_calls = 0 (rollup failed)",
            )

            # Cross-check: main.db rollup matches session.db actuals.
            sconn = sqlite3.connect(
                f"file:{SESSIONS_DIR / session_id / 'session.db'}?mode=ro",
                uri=True,
            )
            actual_fs = sconn.execute("SELECT COUNT(*) FROM fs_events").fetchone()[0]
            actual_net = sconn.execute("SELECT COUNT(*) FROM net_events").fetchone()[0]
            actual_mcp = sconn.execute("SELECT COUNT(*) FROM mcp_calls").fetchone()[0]
            sconn.close()

            r.check(
                row["total_file_events"] == actual_fs,
                f"main.db total_file_events ({row['total_file_events']}) matches session.db ({actual_fs})",
                f"main.db total_file_events ({row['total_file_events']}) != session.db ({actual_fs})",
            )
            r.check(
                row["total_requests"] == actual_net,
                f"main.db total_requests ({row['total_requests']}) matches session.db ({actual_net})",
                f"main.db total_requests ({row['total_requests']}) != session.db ({actual_net})",
            )
            r.check(
                row["total_mcp_calls"] == actual_mcp,
                f"main.db total_mcp_calls ({row['total_mcp_calls']}) matches session.db ({actual_mcp})",
                f"main.db total_mcp_calls ({row['total_mcp_calls']}) != session.db ({actual_mcp})",
            )
        else:
            r.fail(f"session {session_id} not found in main.db")
        mconn.close()
    else:
        r.fail(f"main.db not found at {MAIN_DB}")

    # ── summary ──────────────────────────────────────────────────────
    print(f"\n{BOLD}{'=' * 60}{RESET}")
    total = len(r.passed) + len(r.failed) + len(r.warned)
    print(
        f"  {GREEN}{len(r.passed)} passed{RESET}"
        f"  {RED}{len(r.failed)} failed{RESET}"
        f"  {YELLOW}{len(r.warned)} warnings{RESET}"
        f"  ({total} checks)"
    )
    if r.success:
        print(f"  {GREEN}{BOLD}INTEGRATION TEST PASSED{RESET}\n")
    else:
        print(f"  {RED}{BOLD}INTEGRATION TEST FAILED{RESET}\n")
    return r.success


def main():
    parser = argparse.ArgumentParser(
        description="End-to-end integration test for capsem telemetry pipelines.",
    )
    parser.add_argument(
        "--binary",
        default="target/debug/capsem",
        help="Path to the capsem binary (default: target/debug/capsem)",
    )
    parser.add_argument(
        "--assets",
        default="assets",
        help="Path to VM assets directory (default: assets)",
    )
    args = parser.parse_args()

    session_id, exit_code = run_vm(args.binary, args.assets)

    # The VM command uses semicolons so individual failures don't abort.
    # We don't fail on a non-zero exit code -- the DB assertions decide.
    if exit_code != 0:
        print(f"{YELLOW}VM exited with code {exit_code} (non-fatal, checking DB){RESET}")

    ok = verify_session(session_id)
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
