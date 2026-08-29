"""Validate host-owned rollups, snapshots, and logs for doctor sessions."""

from __future__ import annotations

import json
import sqlite3
from contextlib import closing
from pathlib import Path
from typing import Protocol

BOLD = "\033[1m"
RESET = "\033[0m"


class ResultSink(Protocol):
    def ok(self, message: str) -> None: ...
    def fail(self, message: str) -> None: ...
    def check(self, condition: bool, pass_message: str, fail_message: str) -> None: ...


def _session_counts(db_path: Path) -> tuple[int, int, int]:
    with closing(sqlite3.connect(str(db_path))) as connection:
        file_events = connection.execute("SELECT COUNT(*) FROM fs_events").fetchone()[0]
        requests = connection.execute("SELECT COUNT(*) FROM net_events").fetchone()[0]
        tools = connection.execute("SELECT COUNT(*) FROM tool_calls").fetchone()[0]
    return file_events, requests, tools


def _verify_main_db(
    results: ResultSink,
    session_id: str,
    db_path: Path,
    main_db: Path,
) -> None:
    print(f"\n{BOLD}main.db rollup{RESET}")
    if not main_db.exists():
        results.fail(f"main.db not found at {main_db}")
        return
    with closing(sqlite3.connect(str(main_db))) as connection:
        connection.row_factory = sqlite3.Row
        row = connection.execute("SELECT * FROM sessions WHERE id = ?", (session_id,)).fetchone()
    if row is None:
        results.fail(f"session {session_id} not found in main.db")
        return

    results.check(
        row["status"] in ("stopped", "vacuumed"),
        f"main.db status = {row['status']}",
        f"main.db status = {row['status']} (expected stopped or vacuumed)",
    )
    totals = {
        "total_file_events": row["total_file_events"],
        "total_requests": row["total_requests"],
        "total_tool_calls": row["total_tool_calls"],
    }
    for name, value in totals.items():
        results.check(value > 0, f"main.db {name} = {value}", f"main.db {name} = 0 (rollup failed)")

    actuals = dict(
        zip(
            totals,
            _session_counts(db_path),
            strict=True,
        )
    )
    for name, actual in actuals.items():
        rollup = totals[name]
        results.check(
            rollup == actual,
            f"rollup {name} ({rollup}) matches session.db ({actual})",
            f"rollup {name} ({rollup}) != session.db ({actual})",
        )


def _verify_snapshots(results: ResultSink, session_dir: Path) -> None:
    print(f"\n{BOLD}auto-snapshots{RESET}")
    snapshots = session_dir / "auto_snapshots"
    results.check(
        snapshots.exists(),
        "auto_snapshots directory exists",
        f"auto_snapshots directory NOT found at {snapshots}",
    )
    if not snapshots.exists():
        return
    slot = snapshots / "0"
    results.check(slot.exists(), "boot snapshot slot 0 exists", "boot snapshot slot 0 NOT found")
    if slot.exists():
        workspace = (slot / "workspace").exists()
        system = (slot / "system").exists()
        results.check(
            workspace and system,
            "slot 0 contains workspace/ and system/ subdirectories",
            f"slot 0 missing subdirs (workspace={workspace}, system={system})",
        )


def _valid_log_entry(line: str) -> bool:
    try:
        entry = json.loads(line)
    except json.JSONDecodeError:
        return False
    message = entry.get("message")
    if message is None and "fields" in entry:
        message = entry["fields"].get("message")
    return all(key in entry for key in ("timestamp", "level", "target")) and message is not None


def _verify_log(results: ResultSink, session_dir: Path) -> None:
    print(f"\n{BOLD}log files{RESET}")
    path = session_dir / "process.log"
    results.check(
        path.exists(), f"process.log exists at {path}", f"process.log NOT found at {path}"
    )
    if not path.exists():
        return
    lines = [line for line in path.read_text().splitlines() if line.strip()]
    results.check(
        len(lines) >= 3,
        f"{len(lines)} entries in process.log",
        f"only {len(lines)} entries in process.log (expected >= 3)",
    )
    valid = sum(_valid_log_entry(line) for line in lines)
    results.check(
        valid == len(lines),
        f"all {valid} process.log entries are valid JSONL",
        f"{valid}/{len(lines)} valid JSONL entries",
    )


def verify_host_artifacts(
    results: ResultSink,
    session_id: str,
    session_dir: Path,
    db_path: Path,
    main_db: Path,
) -> None:
    """Validate every host-side artifact after the session DB is closed."""
    _verify_main_db(results, session_id, db_path, main_db)
    _verify_snapshots(results, session_dir)
    _verify_log(results, session_dir)
