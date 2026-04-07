#!/usr/bin/env python3
"""Validate the session DB produced by a capsem-doctor run.

Boots the VM with capsem-doctor, captures the session ID, then inspects
the session.db and main.db to verify that all telemetry pipelines recorded
data correctly during the diagnostic run.

Capsem-doctor exercises network (allowed + denied domains), filesystem
(test file writes), and MCP (tool discovery + invocation) -- but NOT
AI model calls. This test validates that all of those events were captured.

Usage:
    python3 scripts/doctor_session_test.py              # uses target/debug/capsem
    python3 scripts/doctor_session_test.py --binary ./capsem --assets ./assets
"""

import argparse
import gzip
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

SESSIONS_DIR = Path.home() / ".capsem" / "run" / "sessions"
MAIN_DB = Path.home() / ".capsem" / "sessions" / "main.db"


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


def run_doctor(binary: str, assets_dir: str) -> tuple[str, int]:
    """Boot the VM with capsem-doctor, return (session_id, exit_code).

    Finds the session by looking for the newest run-* dir created during
    this invocation (the service preserves session dirs after `capsem run`).
    """
    env = {
        **os.environ,
        "CAPSEM_ASSETS_DIR": assets_dir,
        "RUST_LOG": "capsem=warn",
    }

    # Snapshot existing session dirs so we can diff after.
    existing = set(p.name for p in SESSIONS_DIR.iterdir()) if SESSIONS_DIR.exists() else set()

    print(f"{BOLD}Booting VM with capsem-doctor ...{RESET}")
    proc = subprocess.run(
        [binary, "run", "capsem-doctor"],
        env=env,
        capture_output=True,
        text=True,
        timeout=180,
    )
    exit_code = proc.returncode
    if proc.stdout.strip():
        print(proc.stdout.strip())

    # Find the new session dir.
    new_sessions = sorted(
        (p for p in SESSIONS_DIR.iterdir() if p.name not in existing and p.name.startswith("run-")),
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


def verify_session(session_id: str) -> bool:
    """Open the session DB, run all assertions, return True on success."""
    db_path = SESSIONS_DIR / session_id / "session.db"
    gz_path = SESSIONS_DIR / session_id / "session.db.gz"

    # Session DB may be gzip-compressed after vacuum.
    if not db_path.exists() and gz_path.exists():
        with gzip.open(gz_path, "rb") as f_in:
            db_path.write_bytes(f_in.read())

    if not db_path.exists():
        print(f"{RED}session.db not found at {db_path}{RESET}")
        return False

    conn = sqlite3.connect(str(db_path))
    conn.row_factory = sqlite3.Row
    r = Results()

    # -- net_events --------------------------------------------------------
    print(f"\n{BOLD}net_events{RESET}")
    net_count = conn.execute("SELECT COUNT(*) FROM net_events").fetchone()[0]
    r.check(
        net_count > 0,
        f"{net_count} net_events recorded",
        "no net_events recorded (MITM proxy may not be logging)",
    )

    # capsem-doctor test_network.py makes requests to allowed domains.
    with_status = conn.execute(
        "SELECT COUNT(*) FROM net_events WHERE status_code IS NOT NULL AND status_code > 0"
    ).fetchone()[0]
    r.check(
        with_status >= 1,
        f"{with_status} net_events have HTTP status codes",
        "no net_events with HTTP status codes",
    )

    # Both allowed and denied decisions should be present.
    decisions = conn.execute(
        "SELECT decision, COUNT(*) as cnt FROM net_events GROUP BY decision"
    ).fetchall()
    decision_map = {row["decision"]: row["cnt"] for row in decisions}
    r.check(
        "allowed" in decision_map,
        f"allowed net_events: {decision_map.get('allowed', 0)}",
        "no allowed net_events (test_network allowed-domain tests may have failed)",
    )
    r.check(
        "denied" in decision_map,
        f"denied net_events: {decision_map.get('denied', 0)}",
        "no denied net_events (test_network blocked-domain tests may have failed)",
    )

    # -- fs_events ---------------------------------------------------------
    print(f"\n{BOLD}fs_events{RESET}")
    fs_count = conn.execute("SELECT COUNT(*) FROM fs_events").fetchone()[0]
    r.check(
        fs_count > 0,
        f"{fs_count} fs_events recorded",
        "no fs_events recorded (FS monitor may not be running)",
    )

    # capsem-doctor writes test files -- check for action types.
    if fs_count > 0:
        actions = conn.execute(
            "SELECT action, COUNT(*) as cnt FROM fs_events GROUP BY action"
        ).fetchall()
        action_map = {row["action"]: row["cnt"] for row in actions}
        r.check(
            "modified" in action_map,
            f"modified fs_events: {action_map.get('modified', 0)}",
            "no 'modified' fs_events (capsem-doctor test_workflows writes files)",
        )
        # deleted events may or may not be present depending on test execution
        if "deleted" in action_map:
            r.ok(f"deleted fs_events: {action_map['deleted']}")

    # -- mcp_calls ---------------------------------------------------------
    print(f"\n{BOLD}mcp_calls{RESET}")
    mcp_count = conn.execute("SELECT COUNT(*) FROM mcp_calls").fetchone()[0]
    r.check(
        mcp_count > 0,
        f"{mcp_count} mcp_calls recorded",
        "no mcp_calls recorded (MCP gateway may not be logging)",
    )

    if mcp_count > 0:
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

    # -- model_calls (should be empty) -------------------------------------
    print(f"\n{BOLD}model_calls (regression check){RESET}")
    model_count = conn.execute("SELECT COUNT(*) FROM model_calls").fetchone()[0]
    r.check(
        model_count == 0,
        "0 model_calls (capsem-doctor does not call LLMs)",
        f"{model_count} model_calls found (regression: something is misidentifying traffic as LLM calls)",
    )

    # -- tool_calls / tool_responses (should be empty) ---------------------
    print(f"\n{BOLD}tool_calls / tool_responses (regression check){RESET}")
    tc_count = conn.execute("SELECT COUNT(*) FROM tool_calls").fetchone()[0]
    tr_count = conn.execute("SELECT COUNT(*) FROM tool_responses").fetchone()[0]
    r.check(
        tc_count == 0,
        "0 tool_calls (no AI agent tool use in capsem-doctor)",
        f"{tc_count} tool_calls found (regression)",
    )
    r.check(
        tr_count == 0,
        "0 tool_responses (no AI agent tool use in capsem-doctor)",
        f"{tr_count} tool_responses found (regression)",
    )

    conn.close()

    # -- main.db rollup ----------------------------------------------------
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
                f"rollup total_file_events ({row['total_file_events']}) matches session.db ({actual_fs})",
                f"rollup total_file_events ({row['total_file_events']}) != session.db ({actual_fs})",
            )
            r.check(
                row["total_requests"] == actual_net,
                f"rollup total_requests ({row['total_requests']}) matches session.db ({actual_net})",
                f"rollup total_requests ({row['total_requests']}) != session.db ({actual_net})",
            )
            r.check(
                row["total_mcp_calls"] == actual_mcp,
                f"rollup total_mcp_calls ({row['total_mcp_calls']}) matches session.db ({actual_mcp})",
                f"rollup total_mcp_calls ({row['total_mcp_calls']}) != session.db ({actual_mcp})",
            )
        else:
            r.fail(f"session {session_id} not found in main.db")
        mconn.close()
    else:
        r.fail(f"main.db not found at {MAIN_DB}")

    # -- auto-snapshots ----------------------------------------------------
    print(f"\n{BOLD}auto-snapshots{RESET}")
    snap_dir = SESSIONS_DIR / session_id / "auto_snapshots"
    r.check(
        snap_dir.exists(),
        "auto_snapshots directory exists",
        f"auto_snapshots directory NOT found at {snap_dir}",
    )
    if snap_dir.exists():
        slot0 = snap_dir / "0"
        r.check(
            slot0.exists(),
            "boot snapshot slot 0 exists",
            "boot snapshot slot 0 NOT found",
        )
        if slot0.exists():
            has_workspace = (slot0 / "workspace").exists()
            has_system = (slot0 / "system").exists()
            r.check(
                has_workspace and has_system,
                "slot 0 contains workspace/ and system/ subdirectories",
                f"slot 0 missing subdirs (workspace={has_workspace}, system={has_system})",
            )

    # -- log files ---------------------------------------------------------
    print(f"\n{BOLD}log files{RESET}")
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

        # Verify all lines are valid JSON.
        valid_json = 0
        for line in vm_log_lines:
            try:
                entry = json.loads(line)
                if all(k in entry for k in ("timestamp", "level", "target", "message")):
                    valid_json += 1
            except json.JSONDecodeError:
                pass
        r.check(
            valid_json == len(vm_log_lines),
            f"all {valid_json} process.log entries are valid JSONL",
            f"{valid_json}/{len(vm_log_lines)} valid JSONL entries",
        )

    # -- summary -----------------------------------------------------------
    print(f"\n{BOLD}{'=' * 60}{RESET}")
    total = len(r.passed) + len(r.failed) + len(r.warned)
    print(
        f"  {GREEN}{len(r.passed)} passed{RESET}"
        f"  {RED}{len(r.failed)} failed{RESET}"
        f"  {YELLOW}{len(r.warned)} warnings{RESET}"
        f"  ({total} checks)"
    )
    if r.success:
        print(f"  {GREEN}{BOLD}DOCTOR SESSION VALIDATION PASSED{RESET}\n")
    else:
        print(f"  {RED}{BOLD}DOCTOR SESSION VALIDATION FAILED{RESET}\n")
    return r.success


def main():
    parser = argparse.ArgumentParser(
        description="Validate session DB produced by capsem-doctor run.",
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

    session_id, exit_code = run_doctor(args.binary, args.assets)

    # capsem-doctor must pass -- a failure is itself a test failure.
    if exit_code != 0:
        print(f"{RED}FAIL: capsem-doctor exited with code {exit_code}{RESET}")
        print("capsem-doctor must pass before session validation can proceed.")
        sys.exit(1)
    print(f"  {GREEN}PASS{RESET}  capsem-doctor exited with code 0")

    ok = verify_session(session_id)
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
