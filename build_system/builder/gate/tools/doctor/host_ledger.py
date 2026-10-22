"""Read the host ledger: what the service did to every session.

The service records session and service lifecycle events in database insertion
order. These helpers read the records read-only and validate their shape;
same-owner checksums are not independent proof of tamper resistance.
"""

from __future__ import annotations

import sqlite3
from contextlib import closing
from pathlib import Path
from typing import Any

SESSION_CREATED = "session_created"
SESSION_STOPPED = "session_stopped"


def host_ledger_path(capsem_home: Path) -> Path:
    return capsem_home / "sessions" / "host.db"


def read_host_events(path: Path) -> list[dict[str, Any]]:
    """Every host event in database insertion order."""
    with closing(sqlite3.connect(f"file:{path}?mode=ro", uri=True)) as connection:
        connection.row_factory = sqlite3.Row
        rows = connection.execute(
            "SELECT timestamp_unix_ms, kind, session_id, actor, detail, trace_id"
            " FROM host_events ORDER BY id"
        ).fetchall()
    return [dict(row) for row in rows]


def validate_events(events: list[dict[str, Any]]) -> int:
    """Validate binary event details without pretending to authenticate history."""
    for event in events:
        if not isinstance(event["detail"], (bytes, bytearray, memoryview)):
            raise ValueError("host event detail must be binary data")
    return len(events)


def recent_sessions(path: Path, limit: int) -> list[dict[str, Any]]:
    """Sessions newest first: id, when created and stopped, and status."""
    sessions: dict[str, dict[str, Any]] = {}
    for event in read_host_events(path):
        session_id = event["session_id"]
        if event["kind"] == SESSION_CREATED and session_id:
            sessions[session_id] = {
                "id": session_id,
                "created_at_ms": event["timestamp_unix_ms"],
                "stopped_at_ms": None,
                "status": "running",
            }
        elif event["kind"] == SESSION_STOPPED and session_id in sessions:
            sessions[session_id]["stopped_at_ms"] = event["timestamp_unix_ms"]
            sessions[session_id]["status"] = "stopped"
    ordered = sorted(sessions.values(), key=lambda session: session["created_at_ms"], reverse=True)
    return ordered[:limit]
