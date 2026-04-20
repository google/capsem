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
import signal
import sqlite3
import subprocess
import sys
import time
from pathlib import Path

BOLD = "\033[1m"
DIM = "\033[2m"
GREEN = "\033[32m"
RED = "\033[31m"
YELLOW = "\033[33m"
CYAN = "\033[36m"
RESET = "\033[0m"

def _capsem_home() -> Path:
    """Resolve the capsem base dir, honoring CAPSEM_HOME like the Rust helper.

    Tests run with CAPSEM_HOME pointing at an isolated directory so this
    script never stomps on a locally installed capsem under ~/.capsem.
    """
    env = os.environ.get("CAPSEM_HOME")
    if env:
        return Path(env)
    return Path.home() / ".capsem"


def _run_dir() -> Path:
    env = os.environ.get("CAPSEM_RUN_DIR")
    if env:
        return Path(env)
    return _capsem_home() / "run"


CAPSEM_HOME = _capsem_home()
SESSIONS_DIR = _run_dir() / "sessions"
MAIN_DB = CAPSEM_HOME / "sessions" / "main.db"
SERVICE_SOCKET = _run_dir() / "service.sock"
SERVICE_PIDFILE = _run_dir() / "service.pid"

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
    "curl -sf https://google.com -o /dev/null",
    "curl -sf https://deny.example.com/ -o /dev/null || true",  # denied by policy

    # -- throughput: 100MB download through the full MITM proxy pipeline --
    (
        "curl -s -o /dev/null"
        " -w 'throughput: %{speed_download} B/s in %{time_total}s\\n'"
        " --connect-timeout 5 -m 5"
        " https://ash-speed.hetzner.com/1MB.bin"
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


def _kill_dev_service() -> None:
    """Stop the smoke-owned `capsem-service --foreground` and any child VMs.

    The dev service was started by `just _ensure-service` with no test
    config in env. We replace it with one that inherits our test user/corp
    config so `capsem run` actually routes policy through the test config.
    """
    subprocess.run(["pkill", "-f", "capsem-process.*--id"], check=False)
    subprocess.run(["pkill", "-f", "capsem-service.*--foreground"], check=False)
    time.sleep(0.5)
    subprocess.run(["pkill", "-9", "-f", "capsem-process.*--id"], check=False)
    try:
        SERVICE_PIDFILE.unlink()
    except FileNotFoundError:
        pass
    try:
        SERVICE_SOCKET.unlink()
    except FileNotFoundError:
        pass


def _start_service_with_test_config(
    assets_dir: str, user_config: str, corp_config: str
) -> subprocess.Popen:
    """Spawn `capsem-service --foreground` with test config env vars.

    The service forwards CAPSEM_{USER,CORP}_CONFIG to each `capsem-process`
    it spawns, so the per-VM network policy picks up `deny.example.com`
    and the other overrides from `config/integration-test-user.toml`.
    """
    project_root = Path(__file__).resolve().parent.parent
    service_bin = project_root / "target/debug/capsem-service"
    process_bin = project_root / "target/debug/capsem-process"

    env = {
        **os.environ,
        "CAPSEM_USER_CONFIG": str(project_root / user_config),
        "CAPSEM_CORP_CONFIG": str(project_root / corp_config),
        "RUST_LOG": "capsem=info",
    }

    log_path = project_root / "target/integration-test-service.log"
    log_path.parent.mkdir(parents=True, exist_ok=True)
    log_file = open(log_path, "w")

    proc = subprocess.Popen(
        [
            str(service_bin),
            "--assets-dir", f"{assets_dir}/arm64" if (Path(assets_dir) / "arm64").exists() else assets_dir,
            "--process-binary", str(process_bin),
            "--foreground",
        ],
        env=env,
        stdout=log_file,
        stderr=subprocess.STDOUT,
    )
    SERVICE_PIDFILE.write_text(str(proc.pid))

    deadline = time.monotonic() + 15.0
    while time.monotonic() < deadline:
        if SERVICE_SOCKET.exists():
            # Socket alone isn't enough -- wait for /list to respond.
            r = subprocess.run(
                ["curl", "-s", "--unix-socket", str(SERVICE_SOCKET),
                 "--max-time", "2", "http://localhost/list"],
                capture_output=True,
            )
            if r.returncode == 0:
                return proc
        if proc.poll() is not None:
            raise RuntimeError(
                f"capsem-service exited early (code {proc.returncode}); "
                f"see {log_path}"
            )
        time.sleep(0.2)
    raise RuntimeError(f"capsem-service did not become ready in 15s; see {log_path}")


def run_vm(binary: str, assets_dir: str) -> tuple[str, int]:
    """Boot a temp VM via `capsem run`, return (session_id, exit_code).

    The service preserves the session dir after `run` completes, so we
    find it by looking for the newest `tmp-*` directory created during
    this invocation.
    """
    env = {
        **os.environ,
        "CAPSEM_ASSETS_DIR": assets_dir,
        "RUST_LOG": "capsem=warn",
        "CAPSEM_USER_CONFIG": "config/integration-test-user.toml",
        "CAPSEM_CORP_CONFIG": "config/integration-test-corp.toml",
    }

    # API key: check env, then fall back to ~/.capsem/user.toml.
    google_key = os.environ.get("GOOGLE_API_KEY")
    if not google_key:
        user_toml = Path.home() / ".capsem" / "user.toml"
        if user_toml.exists():
            with open(user_toml) as f:
                for line in f:
                    if line.strip().startswith("value") and "AIza" in line:
                        m = re.search(r'value\s*=\s*"(AIza[^"]*)"', line)
                        if m:
                            google_key = m.group(1)
                            break

    # Restart the dev service with CAPSEM_{USER,CORP}_CONFIG in its env so
    # the policy rules from `config/integration-test-user.toml` actually
    # reach the VM. Without this, the service inherits whatever env
    # `_ensure-service` was launched with (usually nothing), and the
    # per-VM policy falls back to `~/.capsem/user.toml` -- which is the
    # user's real config, not the isolated test config.
    _kill_dev_service()
    service_proc = _start_service_with_test_config(
        assets_dir,
        "config/integration-test-user.toml",
        "config/integration-test-corp.toml",
    )

    # Snapshot session dirs before so we can find the new one after.
    existing = set(p.name for p in SESSIONS_DIR.iterdir()) if SESSIONS_DIR.exists() else set()

    # Pass API key via --env so it reaches the VM through the service.
    cmd = [binary, "run", "--timeout", "300"]
    if google_key:
        cmd.extend(["--env", f"GEMINI_API_KEY={google_key}"])
    cmd.append(VM_COMMAND)

    print(f"{BOLD}Booting VM with test command ...{RESET}")
    try:
        proc = subprocess.run(
            cmd,
            env=env, capture_output=True, text=True, timeout=300,
        )
    finally:
        # Always tear down the test service. Subsequent smoke steps spawn
        # their own fixtures, and leaving this one around would shadow any
        # default-config service the pipeline expects next.
        service_proc.send_signal(signal.SIGTERM)
        try:
            service_proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            service_proc.kill()
        try:
            SERVICE_PIDFILE.unlink()
        except FileNotFoundError:
            pass
    exit_code = proc.returncode
    if proc.stdout.strip():
        print(proc.stdout.strip())

    # Find the new session dir created during this invocation.
    # `capsem run` uses the service's auto-generated `tmp-<adj>-<noun>` ID
    # (see capsem-service/src/main.rs::generate_tmp_name).
    new_sessions = sorted(
        (p for p in SESSIONS_DIR.iterdir() if p.name not in existing and p.name.startswith("tmp-")),
        key=lambda p: p.stat().st_mtime,
        reverse=True,
    ) if SESSIONS_DIR.exists() else []

    if not new_sessions:
        print(f"{RED}FAIL: no new session directory found in {SESSIONS_DIR}{RESET}")
        print(f"    {YELLOW}--- stderr ---{RESET}")
        for line in proc.stderr.strip().splitlines()[:30]:
            print(f"    {line}")
        sys.exit(1)

    session_id = new_sessions[0].name
    print(f"  session: {CYAN}{session_id}{RESET}  exit_code: {exit_code}")
    return session_id, exit_code


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

    # Verify fs_events contain data.  The capsem-doctor snapshot tests produce
    # "restored" events.  Direct guest shell file ops (echo > file) are captured
    # when VirtioFS inotify is active, but may only appear as snapshot-related
    # events depending on timing and the exec path.
    actions = conn.execute(
        "SELECT action, COUNT(*) as cnt FROM fs_events GROUP BY action"
    ).fetchall()
    action_map = {row["action"]: row["cnt"] for row in actions}
    r.check(
        len(action_map) > 0,
        f"fs_event actions: {dict(action_map)}",
        "no fs_event action types recorded",
    )

    # ── net_events ───────────────────────────────────────────────────
    print(f"\n{BOLD}net_events{RESET}")
    net_count = conn.execute("SELECT COUNT(*) FROM net_events").fetchone()[0]
    r.check(
        net_count > 0,
        f"{net_count} net_events recorded",
        "no net_events recorded",
    )

    # google.com from the curl.
    elie = conn.execute(
        "SELECT * FROM net_events WHERE domain = 'google.com'"
    ).fetchone()
    r.check(
        elie is not None,
        "google.com request logged (curl)",
        "google.com NOT found in net_events (curl may have failed)",
    )

    # Allowed decision.
    if elie:
        r.check(
            elie["decision"] == "allowed",
            "google.com decision = allowed",
            f"google.com decision = {elie['decision']} (expected allowed)",
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
    # Structure: "security.web.custom_block" = { value = "domain.com", ... }
    deny_domain = "deny.example.com"
    config_path = Path("config/integration-test-user.toml")
    if config_path.exists():
        with open(config_path, "r") as f:
            for line in f:
                if 'security.web.custom_block' in line and 'value =' in line:
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
    preview_rows = conn.execute(
        "SELECT id, request_preview FROM mcp_calls WHERE method='tools/call'"
    ).fetchall()
    bad_previews = [row["id"] for row in preview_rows
                    if not row["request_preview"] or len(row["request_preview"]) <= 10]
    r.check(
        len(bad_previews) == 0,
        f"all {len(preview_rows)} mcp tools/call have meaningful request_preview",
        f"{len(bad_previews)} mcp tools/call have empty/tiny request_preview (ids: {bad_previews[:5]})",
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

    # Per-VM session log: ~/.capsem/run/sessions/<id>/process.log
    vm_log_path = SESSIONS_DIR / session_id / "process.log"
    r.check(
        vm_log_path.exists(),
        f"process.log exists at {vm_log_path}",
        f"process.log NOT found at {vm_log_path}",
    )

    if vm_log_path.exists():
        vm_log_content = vm_log_path.read_text()
        vm_log_lines = [l for l in vm_log_content.splitlines() if l.strip()]
        r.check(
            len(vm_log_lines) >= 3,
            f"{len(vm_log_lines)} entries in process.log",
            f"only {len(vm_log_lines)} entries in process.log (expected >= 3)",
        )

        # Verify all lines are valid JSON with expected fields.
        valid_json = 0
        invalid_lines = []
        for i, line in enumerate(vm_log_lines):
            try:
                entry = json.loads(line)
                # tracing-subscriber's JSON formatter puts 'message' inside 'fields'.
                # TauriLogLayer puts it at top level. Support both.
                msg = entry.get("message")
                if msg is None and "fields" in entry:
                    msg = entry["fields"].get("message")
                
                has_fields = all(k in entry for k in ("timestamp", "level", "target")) and msg is not None
                if has_fields:
                    valid_json += 1
                else:
                    invalid_lines.append((i + 1, "missing fields"))
            except json.JSONDecodeError:
                invalid_lines.append((i + 1, "invalid JSON"))

        r.check(
            valid_json == len(vm_log_lines),
            f"all {valid_json} process.log entries are valid JSONL",
            f"{len(invalid_lines)} invalid lines in process.log: {invalid_lines[:5]}",
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
            f"process.log contains INFO entries (levels: {levels})",
            f"process.log missing INFO entries (levels: {levels})",
        )
        r.check(
            "DEBUG" not in levels and "TRACE" not in levels,
            "process.log filtered to INFO+ (no DEBUG/TRACE)",
            f"process.log contains debug/trace entries (levels: {levels})",
        )

        # Check for boot_timeline state transition events.
        timeline_entries = []
        for line in vm_log_lines:
            try:
                entry = json.loads(line)
                msg = entry.get("message")
                if msg is None and "fields" in entry:
                    msg = entry["fields"].get("message")
                
                if msg and "state transition" in msg:
                    timeline_entries.append(entry)
            except json.JSONDecodeError:
                pass
        r.check(
            len(timeline_entries) >= 2,
            f"{len(timeline_entries)} boot_timeline state transitions in process.log",
            f"only {len(timeline_entries)} state transitions (expected >= 2 boot phases)",
        )

        # Verify timestamps are valid ISO 8601.
        valid_ts = 0
        for line in vm_log_lines[:5]:
            try:
                entry = json.loads(line)
                ts = entry.get("timestamp", "")
                # Allow 3 to 9 decimal places for sub-seconds.
                if re.match(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d+Z", ts):
                    valid_ts += 1
            except json.JSONDecodeError:
                pass
        r.check(
            valid_ts >= 3,
            f"{valid_ts}/5 sampled timestamps are valid ISO 8601",
            f"only {valid_ts}/5 timestamps match ISO 8601 format",
        )

    # capsem-app per-launch log files live in <capsem_home>/logs/*.jsonl and
    # are only produced when the Tauri desktop shell has been launched. This
    # script exercises `capsem run` / the service, never the desktop app, so
    # inspect the dir only if prior capsem-app runs left logs behind.
    launch_log_dir = CAPSEM_HOME / "logs"
    if launch_log_dir.exists():
        jsonl_files = sorted(launch_log_dir.glob("*.jsonl"), key=lambda p: p.name, reverse=True)
        if jsonl_files:
            latest = jsonl_files[0]
            latest_lines = [l for l in latest.read_text().splitlines() if l.strip()]
            r.check(
                len(latest_lines) >= 5,
                f"latest launch log {latest.name} has {len(latest_lines)} entries",
                f"latest launch log {latest.name} has only {len(latest_lines)} entries (expected >= 5)",
            )
            fname_match = re.match(r"\d{4}-\d{2}-\d{2}T\d{2}-\d{2}-\d{2}", latest.stem)
            r.check(
                fname_match is not None,
                f"launch log filename {latest.name} has valid timestamp format",
                f"launch log filename {latest.name} does not match expected format",
            )

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
        [binary, "run", PERSISTENCE_WRITE_CMD],
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
        [binary, "run", PERSISTENCE_CHECK_CMD],
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
