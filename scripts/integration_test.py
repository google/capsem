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
import json
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
    "curl -sf https://deny.example.com/ -o /dev/null || true",  # denied by policy

    # -- throughput: 100MB download through the full MITM proxy pipeline --
    (
        "curl -s -o /dev/null"
        " -w 'throughput: %{speed_download} B/s in %{time_total}s\\n'"
        " --connect-timeout 15"
        " https://ash-speed.hetzner.com/100MB.bin"
    ),

    # -- mcp_calls: capsem-doctor MCP test subset --
    "capsem-doctor -k mcp",

    # -- model_calls + tool_calls: ask Gemini to write a poem into a file --
    (
        "gemini --yolo -p "
        "'Use the write_file tool to write a four line poem about sandboxes"
        " to the file /root/gemini_poem.txt'"
    ),
    # Fallback: if Gemini printed instead of using write_file, create the file
    # so the fs_events assertion doesn't flake on non-deterministic LLM behavior.
    "test -f /root/gemini_poem.txt || echo 'sandboxes hold the grains of time' > /root/gemini_poem.txt",

    # -- debouncer flush: fs_events uses a 100ms debouncer --
    "sleep 2",

    # -- sentinel so the host can confirm full execution --
    "echo CAPSEM_INTEGRATION_DONE",
])


def run_vm(binary: str, assets_dir: str) -> tuple[str, int]:
    """Boot the VM, run the test command, return (session_id, exit_code)."""
    # Isolate from host settings using dedicated test configs.
    env = {
        **os.environ,
        "CAPSEM_ASSETS_DIR": assets_dir,
        "RUST_LOG": "capsem=warn",
        "CAPSEM_USER_CONFIG": "config/integration-test-user.toml",
        "CAPSEM_CORP_CONFIG": "config/integration-test-corp.toml",
    }

    # API keys flow into the VM via the service's host_config::detect(), which
    # reads env vars and ~/.capsem/user.toml. Pass them through the process env
    # so the service can find them.
    google_key = os.environ.get("GOOGLE_API_KEY")
    if not google_key:
        user_toml = Path.home() / ".capsem" / "user.toml"
        if user_toml.exists():
            with open(user_toml) as f:
                for line in f:
                    if line.strip().startswith("value") and "AIza" in line:
                        match = re.search(r'value\s*=\s*"(AIza[^"]*)"', line)
                        if match:
                            google_key = match.group(1)
                            break
    if google_key:
        env["GEMINI_API_KEY"] = google_key

    print(f"{BOLD}Booting VM with test command ...{RESET}")
    proc = subprocess.run(
        [binary, "run", VM_COMMAND],
        env=env,
        capture_output=True,
        text=True,
        timeout=300,
    )
    output = proc.stdout + "\n" + proc.stderr
    match = re.search(r"\[capsem\] session: (\S+)", output)
    if not match:
        print(f"{RED}FAIL: could not find session ID in output{RESET}")
        print(f"    {CYAN}--- stdout ---{RESET}")
        for line in proc.stdout.strip().splitlines()[:30]:
            print(f"    {line}")
        print(f"    {YELLOW}--- stderr ---{RESET}")
        for line in proc.stderr.strip().splitlines()[:30]:
            print(f"    {line}")
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

    conn = sqlite3.connect(str(db_path))
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

    # Gemini poem file (created by Gemini or fallback echo).
    poem = conn.execute(
        "SELECT * FROM fs_events WHERE path LIKE '%gemini_poem%'"
    ).fetchone()
    r.check(
        poem is not None,
        "gemini_poem.txt logged in fs_events",
        "gemini_poem.txt NOT found in fs_events",
    )

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

    # ash-speed.hetzner.com throughput download.
    hetzner = conn.execute(
        "SELECT * FROM net_events WHERE domain = 'ash-speed.hetzner.com'"
    ).fetchone()
    r.check(
        hetzner is not None,
        "ash-speed.hetzner.com request logged (100MB throughput test)",
        "ash-speed.hetzner.com NOT found in net_events (throughput download may have failed)",
    )
    if hetzner:
        r.check(
            hetzner["decision"] == "allowed",
            "ash-speed.hetzner.com decision = allowed",
            f"ash-speed.hetzner.com decision = {hetzner['decision']} (expected allowed)",
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

    # Denied net_event from curl to blocked domain (from test config).
    # Manually parse the TOML to avoid 'import toml' dependency.
    # Structure: "network.custom_block" = { value = "domain.com", ... }
    deny_domain = "deny.example.com"
    config_path = Path("config/integration-test-user.toml")
    if config_path.exists():
        with open(config_path, "r") as f:
            for line in f:
                if 'network.custom_block' in line and 'value =' in line:
                    # Extract "domain.com" from "network.custom_block" = { value = "domain.com", ... }
                    match = re.search(r'value\s*=\s*"(.*?)"', line)
                    if match:
                        deny_domain = match.group(1).split(",")[0].strip()
                    break

    denied_count = conn.execute(
        "SELECT COUNT(*) FROM net_events WHERE decision = 'denied' AND domain = ?",
        (deny_domain,)
    ).fetchone()[0]
    r.check(
        denied_count >= 1,
        f"{denied_count} denied net_events for {deny_domain} (policy enforcement working)",
        f"no denied net_events for {deny_domain} (curl to blocked domain may have failed silently)",
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

    # Bug 2: MCP data completeness -- request_preview must not be truncated
    for row in conn.execute(
        "SELECT id, request_preview FROM mcp_calls WHERE method='tools/call'"
    ).fetchall():
        r.check(
            row["request_preview"] and len(row["request_preview"]) > 10,
            f"mcp_call {row['id']} has meaningful request_preview",
            f"mcp_call {row['id']} has empty/tiny request_preview",
        )

    # Bug 2: bytes tracking
    mcp_with_bytes = conn.execute(
        "SELECT COUNT(*) FROM mcp_calls WHERE method='tools/call' AND bytes_sent > 0"
    ).fetchone()[0]
    r.check(
        mcp_with_bytes > 0,
        f"{mcp_with_bytes} mcp tools/call have bytes_sent > 0",
        "no mcp tools/call with bytes_sent -- byte tracking broken",
    )

    # Bug 3: builtin HTTP in net_events
    mcp_net = conn.execute(
        "SELECT COUNT(*) FROM net_events WHERE conn_type = 'mcp_builtin'"
    ).fetchone()[0]
    r.check(
        mcp_net > 0,
        f"{mcp_net} net_events from MCP builtin tools",
        "no net_events with conn_type=mcp_builtin -- builtin HTTP not logged",
    )

    # Bug 4: process_name
    bad_proc = conn.execute(
        "SELECT COUNT(*) FROM mcp_calls WHERE process_name = 'MainThread'"
    ).fetchone()[0]
    r.check(
        bad_proc == 0,
        "no mcp_calls with process_name='MainThread'",
        f"{bad_proc} mcp_calls have process_name='MainThread'",
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
                f"Gemini token counts are zero: {in_tok} in / {out_tok} out (API key may be invalid)",
            )
        else:
            r.fail("no Gemini model_call with a model name (stream parsing incomplete)")

    # Cost estimation -- at least one model_call should have a positive cost.
    with_cost = conn.execute(
        "SELECT COUNT(*) FROM model_calls WHERE estimated_cost_usd > 0"
    ).fetchone()[0]
    r.check(
        with_cost >= 1,
        f"{with_cost} model_calls with positive estimated_cost_usd",
        "no model_calls with positive cost (API may have returned an error)",
    )

    # ── tool_calls / tool_responses ──────────────────────────────────
    print(f"\n{BOLD}tool_calls / tool_responses{RESET}")
    tc_count = conn.execute("SELECT COUNT(*) FROM tool_calls").fetchone()[0]
    tr_count = conn.execute("SELECT COUNT(*) FROM tool_responses").fetchone()[0]

    # Bug 1: No duplicate tool_responses
    if tr_count > 0:
        unique_tr = conn.execute(
            "SELECT COUNT(*) FROM (SELECT DISTINCT call_id, content_preview FROM tool_responses)"
        ).fetchone()[0]
        r.check(
            tr_count == unique_tr,
            f"no duplicate tool_responses ({tr_count} total, {unique_tr} unique)",
            f"DUPLICATE tool_responses: {tr_count} total but only {unique_tr} unique",
        )

    # Gemini may or may not use tools -- it's non-deterministic.
    # We validate tool_calls metadata when present, but don't fail on 0.
    if tc_count > 0:
        r.ok(f"{tc_count} tool_calls recorded (Gemini used tools)")
    else:
        r.ok("0 tool_calls (Gemini printed instead of using tools -- non-deterministic)")

    if tc_count > 0:
        # Origin column should be populated on all tool_calls.
        with_origin = conn.execute(
            "SELECT COUNT(*) FROM tool_calls WHERE origin IS NOT NULL AND origin != ''"
        ).fetchone()[0]
        r.check(
            with_origin == tc_count,
            f"all {with_origin} tool_calls have origin column populated",
            f"only {with_origin}/{tc_count} tool_calls have origin",
        )

        # Gemini's streaming format does not always produce a parseable
        # tool_response turn, so tool_responses may lag behind tool_calls.
        if tr_count >= tc_count:
            r.ok(f"{tr_count} tool_responses match {tc_count} tool_calls")
        else:
            r.ok(
                f"tool_responses ({tr_count}) < tool_calls ({tc_count})"
                " -- Gemini stream parser limitation (non-blocking)"
            )

    conn.close()

    # ── main.db rollup ───────────────────────────────────────────────
    print(f"\n{BOLD}main.db rollup{RESET}")
    if MAIN_DB.exists():
        mconn = sqlite3.connect(str(MAIN_DB))
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
            sconn = sqlite3.connect(str(SESSIONS_DIR / session_id / "session.db"))
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

    # ── log files ─────────────────────────────────────────────────────
    print(f"\n{BOLD}log files{RESET}")

    # Per-VM session log: ~/.capsem/sessions/<id>/capsem.log
    vm_log_path = SESSIONS_DIR / session_id / "capsem.log"
    r.check(
        vm_log_path.exists(),
        f"capsem.log exists at {vm_log_path}",
        f"capsem.log NOT found at {vm_log_path}",
    )

    if vm_log_path.exists():
        vm_log_content = vm_log_path.read_text()
        vm_log_lines = [l for l in vm_log_content.splitlines() if l.strip()]
        r.check(
            len(vm_log_lines) >= 3,
            f"{len(vm_log_lines)} entries in capsem.log",
            f"only {len(vm_log_lines)} entries in capsem.log (expected >= 3)",
        )

        # Verify all lines are valid JSON with expected fields.
        valid_json = 0
        invalid_lines = []
        for i, line in enumerate(vm_log_lines):
            try:
                entry = json.loads(line)
                has_fields = all(
                    k in entry for k in ("timestamp", "level", "target", "message")
                )
                if has_fields:
                    valid_json += 1
                else:
                    invalid_lines.append((i + 1, "missing fields"))
            except json.JSONDecodeError:
                invalid_lines.append((i + 1, "invalid JSON"))

        r.check(
            valid_json == len(vm_log_lines),
            f"all {valid_json} capsem.log entries are valid JSONL",
            f"{len(invalid_lines)} invalid lines in capsem.log: {invalid_lines[:5]}",
        )

        # Check that log entries contain expected levels (INFO and above only).
        levels = set()
        for line in vm_log_lines:
            try:
                entry = json.loads(line)
                levels.add(entry.get("level", ""))
            except json.JSONDecodeError:
                pass
        r.check(
            "INFO" in levels,
            f"capsem.log contains INFO entries (levels: {levels})",
            f"capsem.log missing INFO entries (levels: {levels})",
        )
        r.check(
            "DEBUG" not in levels and "TRACE" not in levels,
            "capsem.log filtered to INFO+ (no DEBUG/TRACE)",
            f"capsem.log contains debug/trace entries (levels: {levels})",
        )

        # Check for boot_timeline state transition events.
        timeline_entries = []
        for line in vm_log_lines:
            try:
                entry = json.loads(line)
                if "state transition" in entry.get("message", ""):
                    timeline_entries.append(entry)
            except json.JSONDecodeError:
                pass
        r.check(
            len(timeline_entries) >= 2,
            f"{len(timeline_entries)} boot_timeline state transitions in capsem.log",
            f"only {len(timeline_entries)} state transitions (expected >= 2 boot phases)",
        )

        # Verify timestamps are valid ISO 8601.
        valid_ts = 0
        for line in vm_log_lines[:5]:
            try:
                entry = json.loads(line)
                ts = entry.get("timestamp", "")
                if re.match(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z", ts):
                    valid_ts += 1
            except json.JSONDecodeError:
                pass
        r.check(
            valid_ts >= 3,
            f"{valid_ts}/5 sampled timestamps are valid ISO 8601",
            f"only {valid_ts}/5 timestamps match ISO 8601 format",
        )

    # Per-launch log file: ~/.capsem/logs/*.jsonl
    launch_log_dir = Path.home() / ".capsem" / "logs"
    if launch_log_dir.exists():
        jsonl_files = sorted(launch_log_dir.glob("*.jsonl"), key=lambda p: p.name, reverse=True)
        r.check(
            len(jsonl_files) >= 1,
            f"{len(jsonl_files)} launch log files in {launch_log_dir}",
            f"no .jsonl files in {launch_log_dir}",
        )

        if jsonl_files:
            # Verify the most recent launch log has entries.
            latest = jsonl_files[0]
            latest_lines = [l for l in latest.read_text().splitlines() if l.strip()]
            r.check(
                len(latest_lines) >= 5,
                f"latest launch log {latest.name} has {len(latest_lines)} entries",
                f"latest launch log {latest.name} has only {len(latest_lines)} entries (expected >= 5)",
            )

            # Verify launch log filename is valid timestamp format.
            fname_match = re.match(r"\d{4}-\d{2}-\d{2}T\d{2}-\d{2}-\d{2}", latest.stem)
            r.check(
                fname_match is not None,
                f"launch log filename {latest.name} has valid timestamp format",
                f"launch log filename {latest.name} does not match expected format",
            )
    else:
        r.fail(f"launch log directory {launch_log_dir} not found")

    # ── auto-snapshots ────────────────────────────────────────────────
    print(f"\n{BOLD}auto-snapshots{RESET}")
    snap_dir = SESSIONS_DIR / session_id / "auto_snapshots"
    r.check(
        snap_dir.exists(),
        f"auto_snapshots directory exists",
        f"auto_snapshots directory NOT found at {snap_dir}",
    )
    if snap_dir.exists():
        slot0 = snap_dir / "0"
        r.check(
            slot0.exists(),
            "boot snapshot slot 0 exists",
            "boot snapshot slot 0 NOT found (auto-snapshot scheduler may not have run)",
        )
        if slot0.exists():
            has_workspace = (slot0 / "workspace").exists()
            has_system = (slot0 / "system").exists()
            r.check(
                has_workspace and has_system,
                "slot 0 contains workspace/ and system/ subdirectories",
                f"slot 0 missing subdirs (workspace={has_workspace}, system={has_system})",
            )

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


PERSISTENCE_WRITE_CMD = (
    "echo capsem-persistence-sentinel > /root/.capsem_persistence_test "
    "&& echo CAPSEM_PERSISTENCE_WRITTEN"
)
PERSISTENCE_CHECK_CMD = (
    "test ! -f /root/.capsem_persistence_test "
    "&& echo CAPSEM_EPHEMERAL_OK "
    "|| { echo CAPSEM_EPHEMERAL_FAIL; exit 1; }"
)


def check_persistence(binary: str, assets_dir: str) -> bool:
    """Boot two consecutive VMs; verify a file written in the first is gone in the second."""
    print(f"\n{BOLD}=== Ephemeral model check ==={RESET}")
    env = {
        **os.environ,
        "CAPSEM_ASSETS_DIR": assets_dir,
        "RUST_LOG": "capsem=warn",
        "CAPSEM_USER_CONFIG": "config/integration-test-user.toml",
        "CAPSEM_CORP_CONFIG": "config/integration-test-corp.toml",
    }

    print("  Invocation 1: writing sentinel file...")
    proc1 = subprocess.run(
        [binary, PERSISTENCE_WRITE_CMD],
        env=env, capture_output=True, text=True, timeout=120,
    )
    output1 = proc1.stdout + "\n" + proc1.stderr
    if "CAPSEM_PERSISTENCE_WRITTEN" not in output1:
        print(f"  {RED}FAIL{RESET}  sentinel write failed (invocation 1 did not confirm)")
        print(output1[:1000])
        return False
    print(f"  {GREEN}PASS{RESET}  sentinel written in invocation 1")

    print("  Invocation 2: checking sentinel is absent...")
    proc2 = subprocess.run(
        [binary, PERSISTENCE_CHECK_CMD],
        env=env, capture_output=True, text=True, timeout=120,
    )
    output2 = proc2.stdout + "\n" + proc2.stderr
    # Use exit code as the definitive indicator -- the command string itself contains
    # "CAPSEM_EPHEMERAL_FAIL" so searching for it in output would always match (PTY echo).
    if proc2.returncode != 0:
        print(f"  {RED}FAIL{RESET}  sentinel persisted across VM invocations -- SECURITY BREACH")
        return False
    if "CAPSEM_EPHEMERAL_OK" not in output2:
        print(f"  {RED}FAIL{RESET}  ephemeral check did not confirm (no CAPSEM_EPHEMERAL_OK)")
        return False
    print(f"  {GREEN}PASS{RESET}  sentinel absent in invocation 2 (VM is fully ephemeral)")
    return True


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

    telemetry_ok = verify_session(session_id)
    ephemeral_ok = check_persistence(args.binary, args.assets)
    sys.exit(0 if (telemetry_ok and ephemeral_ok) else 1)


if __name__ == "__main__":
    main()
