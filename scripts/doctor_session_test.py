#!/usr/bin/env python3
"""Validate the session DB produced by a capsem-doctor run.

Boots the VM with capsem-doctor, captures the session ID, then inspects
the session.db and main.db to verify that all telemetry pipelines recorded
data correctly during the diagnostic run.

Capsem-doctor exercises network (allowed + denied domains), filesystem
(test file writes), MCP (tool discovery + invocation), and hermetic
model-shaped traffic through the local mock server. This test validates
that all of those events were captured.

Usage:
    python3 scripts/doctor_session_test.py              # uses target/debug/capsem
    python3 scripts/doctor_session_test.py --binary ./capsem --assets ./assets

Ironbank note: this script is a black-box ledger validator. Do not weaken it
into status-only checks, row-exists checks, skipped cases, slow/optional cases,
or Rust-internal expectations. Release-critical cases belong in
tests/ironbank/ and must assert the full public ledger.
"""

import argparse
import gzip
import json
import os
import shlex
import sqlite3
import subprocess
import sys
import time
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

from mock_server import start_mock_server, stop_process  # noqa: E402

BOLD = "\033[1m"
DIM = "\033[2m"
GREEN = "\033[32m"
RED = "\033[31m"
YELLOW = "\033[33m"
CYAN = "\033[36m"
RESET = "\033[0m"

PROJECT_ROOT = Path(__file__).resolve().parents[1]
MOCK_SERVER_ENV = "CAPSEM_MOCK_SERVER_BASE_URL"


def _capsem_home() -> Path:
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
PERSISTENT_DIR = _run_dir() / "persistent"
MAIN_DB = CAPSEM_HOME / "sessions" / "main.db"


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


def _new_session_dirs(root: Path, existing: set[str]) -> list[Path]:
    if not root.exists():
        return []
    return sorted(
        (p for p in root.iterdir() if p.is_dir() and p.name not in existing),
        key=lambda p: p.stat().st_mtime,
        reverse=True,
    )


def _parse_created_session_id(stdout: str) -> str:
    for line in stdout.splitlines():
        stripped = line.strip()
        if stripped:
            return stripped.split()[0]
    raise RuntimeError("capsem create returned no session id")


def _cli_env(assets_dir: str) -> dict[str, str]:
    return {
        **os.environ,
        "CAPSEM_ASSETS_DIR": assets_dir,
        "RUST_LOG": "capsem=warn",
    }


def run_doctor(binary: str, assets_dir: str, mock_base_url: str) -> tuple[str, Path, int]:
    """Boot the VM with capsem-doctor, return (session_id, exit_code).

    Uses an explicit named session so the post-run session DB remains
    available for ledger validation. `capsem run` intentionally cleans up its
    ephemeral session directory after exit.
    """
    env = _cli_env(assets_dir)

    existing_persistent = (
        set(p.name for p in PERSISTENT_DIR.iterdir()) if PERSISTENT_DIR.exists() else set()
    )
    existing_sessions = (
        set(p.name for p in SESSIONS_DIR.iterdir()) if SESSIONS_DIR.exists() else set()
    )

    session_name = f"doctor-ledger-{os.getpid()}-{int(time.time())}"
    print(f"{BOLD}Creating VM for capsem-doctor ...{RESET}")
    create = subprocess.run(
        [
            binary,
            "create",
            "-n",
            session_name,
            "--ram",
            "2",
            "--cpu",
            "2",
            "-e",
            f"{MOCK_SERVER_ENV}={mock_base_url}",
        ],
        env=env,
        capture_output=True,
        text=True,
        timeout=180,
    )
    if create.returncode != 0:
        if create.stdout.strip():
            print(create.stdout.strip())
        if create.stderr.strip():
            print(create.stderr.strip(), file=sys.stderr)
        sys.exit(create.returncode)
    session_id = _parse_created_session_id(create.stdout)
    session_dir = PERSISTENT_DIR / session_id

    if not session_dir.exists():
        new_sessions = _new_session_dirs(PERSISTENT_DIR, existing_persistent)
        if new_sessions:
            session_dir = new_sessions[0]
            session_id = session_dir.name

    if not session_dir.exists():
        print(f"{RED}FAIL: no persistent session directory found in {PERSISTENT_DIR}{RESET}")
        print(f"    {YELLOW}--- stderr ---{RESET}")
        for line in create.stderr.strip().splitlines()[:30]:
            print(f"    {line}")
        sys.exit(1)

    print(f"{BOLD}Booting VM with capsem-doctor ...{RESET}")
    proc = subprocess.run(
        [
            binary,
            "exec",
            session_id,
            (
                f"export {MOCK_SERVER_ENV}={shlex.quote(mock_base_url)}; "
                "capsem-doctor"
            ),
            "--timeout",
            "220",
        ],
        env=env,
        capture_output=True,
        text=True,
        timeout=240,
    )
    exit_code = proc.returncode
    if proc.stdout.strip():
        print(proc.stdout.strip())
    if proc.stderr.strip():
        print(proc.stderr.strip(), file=sys.stderr)

    preserved_dir = cleanup_session(binary, session_id, assets_dir, existing_sessions)
    if preserved_dir is not None:
        session_dir = preserved_dir

    print(f"  session: {CYAN}{session_id}{RESET}  exit_code: {exit_code}")
    return session_id, session_dir, exit_code


def cleanup_session(
    binary: str,
    session_id: str,
    assets_dir: str,
    existing_sessions: set[str],
) -> Path | None:
    subprocess.run(
        [binary, "delete", session_id],
        env=_cli_env(assets_dir),
        capture_output=True,
        text=True,
        timeout=60,
        check=False,
    )
    for _ in range(50):
        matches = [
            p
            for p in _new_session_dirs(SESSIONS_DIR, existing_sessions)
            if p.name.startswith(f"{session_id}-")
        ]
        if matches:
            return matches[0]
        time.sleep(0.1)
    return None


def verify_session(session_id: str, session_dir: Path) -> bool:
    """Open the session DB, run all assertions, return True on success."""
    db_path = session_dir / "session.db"
    gz_path = session_dir / "session.db.gz"

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
    blocked_count = sum(decision_map.get(k, 0) for k in ("denied", "blocked", "error"))
    r.check(
        blocked_count > 0,
        f"blocked/error net_events: {blocked_count}",
        "no blocked/error net_events (test_network blocked-domain tests may have failed)",
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
        write_like_count = sum(action_map.get(k, 0) for k in ("created", "modified", "restored"))
        r.check(
            write_like_count > 0,
            f"write-like fs_events recorded: {write_like_count}",
            "no created/modified/restored fs_events (capsem-doctor file probes may not be logged)",
        )
        # deleted events may or may not be present depending on test execution
        if "deleted" in action_map:
            r.ok(f"deleted fs_events: {action_map['deleted']}")

    # -- MCP-origin tool_calls ---------------------------------------------
    print(f"\n{BOLD}MCP-origin tool_calls{RESET}")
    mcp_table = conn.execute(
        "SELECT name FROM sqlite_master WHERE type='table' AND name='mcp_calls'"
    ).fetchone()
    r.check(
        mcp_table is None,
        "mcp_calls table absent",
        "mcp_calls table still exists; tool invocations must use tool_calls",
    )
    mcp_count = conn.execute(
        "SELECT COUNT(*) FROM tool_calls WHERE origin = 'mcp'"
    ).fetchone()[0]
    r.check(
        mcp_count > 0,
        f"{mcp_count} MCP-origin tool_calls recorded",
        "no MCP-origin tool_calls recorded (guest MCP endpoint may not be logging)",
    )

    # -- model_calls -------------------------------------------------------
    print(f"\n{BOLD}model_calls{RESET}")
    model_count = conn.execute("SELECT COUNT(*) FROM model_calls").fetchone()[0]
    r.check(
        model_count > 0,
        f"{model_count} model_calls recorded",
        "no model_calls recorded (local OpenAI-compatible fixture parsing may have failed)",
    )
    if model_count > 0:
        fixture_model = conn.execute(
            "SELECT * FROM model_calls"
            " WHERE model = 'mock-local'"
            " AND path = '/v1/chat/completions'"
            " ORDER BY id DESC LIMIT 1"
        ).fetchone()
        r.check(
            fixture_model is not None,
            "mock-local OpenAI-compatible model_call recorded",
            "mock-local OpenAI-compatible model_call missing",
        )
        if fixture_model is not None:
            r.check(
                (fixture_model["input_tokens"] or 0) > 0
                and (fixture_model["output_tokens"] or 0) > 0,
                "mock-local model_call has token usage",
                "mock-local model_call missing token usage",
            )

    # -- tool_calls / tool_responses ---------------------------------------
    print(f"\n{BOLD}tool_calls / tool_responses{RESET}")
    tc_count = conn.execute("SELECT COUNT(*) FROM tool_calls").fetchone()[0]
    tr_count = conn.execute("SELECT COUNT(*) FROM tool_responses").fetchone()[0]
    r.check(
        tc_count > 0,
        f"{tc_count} tool_calls recorded",
        "no tool_calls recorded (mock model fixture tool call parsing may have failed)",
    )
    fixture_tool_call = conn.execute(
        "SELECT COUNT(*) FROM tool_calls WHERE tool_name = 'fixture_lookup'"
    ).fetchone()[0]
    r.check(
        fixture_tool_call > 0,
        f"fixture_lookup tool_calls recorded: {fixture_tool_call}",
        "fixture_lookup tool_call missing",
    )
    r.check(
        tr_count == 0,
        "0 tool_responses (fixture emits a request-side tool call only)",
        f"{tr_count} tool_responses found (unexpected)",
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
                row["total_tool_calls"] > 0,
                f"main.db total_tool_calls = {row['total_tool_calls']}",
                "main.db total_tool_calls = 0 (rollup failed)",
            )

            # Cross-check: main.db rollup matches session.db actuals.
            sconn = sqlite3.connect(str(db_path))
            actual_fs = sconn.execute("SELECT COUNT(*) FROM fs_events").fetchone()[0]
            actual_net = sconn.execute("SELECT COUNT(*) FROM net_events").fetchone()[0]
            actual_tools = sconn.execute("SELECT COUNT(*) FROM tool_calls").fetchone()[0]
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
                row["total_tool_calls"] == actual_tools,
                f"rollup total_tool_calls ({row['total_tool_calls']}) matches session.db ({actual_tools})",
                f"rollup total_tool_calls ({row['total_tool_calls']}) != session.db ({actual_tools})",
            )
        else:
            r.fail(f"session {session_id} not found in main.db")
        mconn.close()
    else:
        r.fail(f"main.db not found at {MAIN_DB}")

    # -- auto-snapshots ----------------------------------------------------
    print(f"\n{BOLD}auto-snapshots{RESET}")
    snap_dir = session_dir / "auto_snapshots"
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
    vm_log_path = session_dir / "process.log"
    r.check(
        vm_log_path.exists(),
        f"process.log exists at {vm_log_path}",
        f"process.log NOT found at {vm_log_path}",
    )

    if vm_log_path.exists():
        vm_log_content = vm_log_path.read_text()
        vm_log_lines = [line for line in vm_log_content.splitlines() if line.strip()]
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
                # tracing-subscriber's JSON formatter puts 'message' inside 'fields'.
                # TauriLogLayer puts it at top level. Support both.
                msg = entry.get("message")
                if msg is None and "fields" in entry:
                    msg = entry["fields"].get("message")

                if all(k in entry for k in ("timestamp", "level", "target")) and msg is not None:
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

    mock_proc = None
    try:
        mock_proc, ready = start_mock_server()
        mock_base_url = ready["base_url"]
        print(f"{BOLD}Local mock server:{RESET} {mock_base_url}")
        session_id, session_dir, exit_code = run_doctor(args.binary, args.assets, mock_base_url)
    finally:
        stop_process(mock_proc)

    # capsem-doctor must pass -- a failure is itself a test failure.
    if exit_code != 0:
        print(f"{RED}FAIL: capsem-doctor exited with code {exit_code}{RESET}")
        print("capsem-doctor must pass before session validation can proceed.")
        sys.exit(1)
    print(f"  {GREEN}PASS{RESET}  capsem-doctor exited with code 0")

    ok = verify_session(session_id, session_dir)
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
