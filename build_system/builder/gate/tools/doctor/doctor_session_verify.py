"""Validate the event ledger produced by a completed doctor session."""

from __future__ import annotations

import gzip
import sqlite3
from contextlib import closing
from pathlib import Path

from capsem_builder.gate.tools.doctor.doctor_session_host_verify import (
    verify_host_artifacts,
)

BOLD = "\033[1m"
GREEN = "\033[32m"
RED = "\033[31m"
YELLOW = "\033[33m"
RESET = "\033[0m"


class Results:
    """Accumulate pass, failure, and warning evidence for one validation."""

    def __init__(self) -> None:
        self.passed: list[str] = []
        self.failed: list[str] = []
        self.warned: list[str] = []

    def ok(self, message: str) -> None:
        self.passed.append(message)
        print(f"  {GREEN}PASS{RESET}  {message}")

    def fail(self, message: str) -> None:
        self.failed.append(message)
        print(f"  {RED}FAIL{RESET}  {message}")

    def check(self, condition: bool, pass_message: str, fail_message: str) -> None:
        if condition:
            self.ok(pass_message)
        else:
            self.fail(fail_message)

    @property
    def success(self) -> bool:
        return not self.failed


def _session_database(session_dir: Path) -> Path | None:
    path = session_dir / "session.db"
    compressed = session_dir / "session.db.gz"
    if not path.exists() and compressed.exists():
        with gzip.open(compressed, "rb") as source:
            path.write_bytes(source.read())
    if path.exists():
        return path
    print(f"{RED}session.db not found at {path}{RESET}")
    return None


def _verify_network(connection: sqlite3.Connection, results: Results) -> None:
    print(f"\n{BOLD}net_events{RESET}")
    count = connection.execute("SELECT COUNT(*) FROM net_events").fetchone()[0]
    results.check(
        count > 0,
        f"{count} net_events recorded",
        "no net_events recorded (MITM proxy may not be logging)",
    )
    with_status = connection.execute(
        "SELECT COUNT(*) FROM net_events WHERE status_code IS NOT NULL AND status_code > 0"
    ).fetchone()[0]
    results.check(
        with_status >= 1,
        f"{with_status} net_events have HTTP status codes",
        "no net_events with HTTP status codes",
    )
    rows = connection.execute(
        "SELECT decision, COUNT(*) as cnt FROM net_events GROUP BY decision"
    ).fetchall()
    decisions = {row["decision"]: row["cnt"] for row in rows}
    results.check(
        "allowed" in decisions,
        f"allowed net_events: {decisions.get('allowed', 0)}",
        "no allowed net_events (test_network allowed-domain tests may have failed)",
    )
    blocked = sum(decisions.get(key, 0) for key in ("denied", "blocked", "error"))
    results.check(
        blocked > 0,
        f"blocked/error net_events: {blocked}",
        "no blocked/error net_events (test_network blocked-domain tests may have failed)",
    )


def _verify_filesystem(connection: sqlite3.Connection, results: Results) -> None:
    print(f"\n{BOLD}fs_events{RESET}")
    count = connection.execute("SELECT COUNT(*) FROM fs_events").fetchone()[0]
    results.check(
        count > 0,
        f"{count} fs_events recorded",
        "no fs_events recorded (FS monitor may not be running)",
    )
    if count == 0:
        return
    rows = connection.execute(
        "SELECT action, COUNT(*) as cnt FROM fs_events GROUP BY action"
    ).fetchall()
    actions = {row["action"]: row["cnt"] for row in rows}
    writes = sum(actions.get(key, 0) for key in ("created", "modified", "restored"))
    results.check(
        writes > 0,
        f"write-like fs_events recorded: {writes}",
        "no created/modified/restored fs_events (capsem-doctor file probes may not be logged)",
    )
    if "deleted" in actions:
        results.ok(f"deleted fs_events: {actions['deleted']}")


def _verify_mcp(connection: sqlite3.Connection, results: Results) -> None:
    print(f"\n{BOLD}MCP-origin tool_calls{RESET}")
    retired = connection.execute(
        "SELECT name FROM sqlite_master WHERE type='table' AND name='mcp_calls'"
    ).fetchone()
    results.check(
        retired is None,
        "mcp_calls table absent",
        "mcp_calls table still exists; tool invocations must use tool_calls",
    )
    count = connection.execute("SELECT COUNT(*) FROM tool_calls WHERE origin = 'mcp'").fetchone()[0]
    results.check(
        count > 0,
        f"{count} MCP-origin tool_calls recorded",
        "no MCP-origin tool_calls recorded (guest MCP endpoint may not be logging)",
    )


def _verify_models(connection: sqlite3.Connection, results: Results) -> None:
    print(f"\n{BOLD}model_calls{RESET}")
    count = connection.execute("SELECT COUNT(*) FROM model_calls").fetchone()[0]
    results.check(
        count > 0,
        f"{count} model_calls recorded",
        "no model_calls recorded (local OpenAI-compatible fixture parsing may have failed)",
    )
    if count == 0:
        return
    fixture = connection.execute(
        "SELECT * FROM model_calls WHERE model = 'mock-local'"
        " AND path = '/v1/chat/completions' ORDER BY id DESC LIMIT 1"
    ).fetchone()
    results.check(
        fixture is not None,
        "mock-local OpenAI-compatible model_call recorded",
        "mock-local OpenAI-compatible model_call missing",
    )
    if fixture is not None:
        results.check(
            (fixture["input_tokens"] or 0) > 0 and (fixture["output_tokens"] or 0) > 0,
            "mock-local model_call has token usage",
            "mock-local model_call missing token usage",
        )


def _verify_tools(connection: sqlite3.Connection, results: Results) -> None:
    print(f"\n{BOLD}tool_calls / tool_responses{RESET}")
    calls = connection.execute("SELECT COUNT(*) FROM tool_calls").fetchone()[0]
    responses = connection.execute("SELECT COUNT(*) FROM tool_responses").fetchone()[0]
    results.check(
        calls > 0,
        f"{calls} tool_calls recorded",
        "no tool_calls recorded (mock model fixture tool call parsing may have failed)",
    )
    fixture = connection.execute(
        "SELECT COUNT(*) FROM tool_calls WHERE tool_name = 'fixture_lookup'"
    ).fetchone()[0]
    results.check(
        fixture > 0,
        f"fixture_lookup tool_calls recorded: {fixture}",
        "fixture_lookup tool_call missing",
    )
    results.check(
        responses == 0,
        "0 tool_responses (fixture emits a request-side tool call only)",
        f"{responses} tool_responses found (unexpected)",
    )


def verify_session(
    session_id: str,
    session_dir: Path,
    *,
    main_db: Path,
) -> bool:
    """Open the session DB, run all assertions, and return the verdict."""
    db_path = _session_database(session_dir)
    if db_path is None:
        return False
    results = Results()
    with closing(sqlite3.connect(str(db_path))) as connection:
        connection.row_factory = sqlite3.Row
        _verify_network(connection, results)
        _verify_filesystem(connection, results)
        _verify_mcp(connection, results)
        _verify_models(connection, results)
        _verify_tools(connection, results)
    verify_host_artifacts(results, session_id, session_dir, db_path, main_db)

    print(f"\n{BOLD}{'=' * 60}{RESET}")
    total = len(results.passed) + len(results.failed) + len(results.warned)
    print(
        f"  {GREEN}{len(results.passed)} passed{RESET}"
        f"  {RED}{len(results.failed)} failed{RESET}"
        f"  {YELLOW}{len(results.warned)} warnings{RESET}"
        f"  ({total} checks)"
    )
    verdict = "PASSED" if results.success else "FAILED"
    color = GREEN if results.success else RED
    print(f"  {color}{BOLD}DOCTOR SESSION VALIDATION {verdict}{RESET}\n")
    return results.success
