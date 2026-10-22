"""The doctor tools read the host ledger the way the service writes it."""

from __future__ import annotations

import sqlite3
from contextlib import closing
from pathlib import Path

import pytest
from capsem_builder.gate.tools.doctor.host_ledger import (
    recent_sessions,
    validate_events,
)


def event(kind: str, session: str | None, at: int) -> dict:
    return {
        "timestamp_unix_ms": at,
        "kind": kind,
        "session_id": session,
        "actor": "service",
        "detail": b"\x80",
        "trace_id": None,
    }


def write_ledger(path: Path, events: list[dict]) -> None:
    with closing(sqlite3.connect(path)) as connection:
        connection.execute(
            "CREATE TABLE host_events (id INTEGER PRIMARY KEY, timestamp_unix_ms INTEGER, kind TEXT,"
            " session_id TEXT, actor TEXT, detail BLOB, trace_id TEXT)"
        )
        for item in events:
            connection.execute(
                "INSERT INTO host_events (timestamp_unix_ms, kind, session_id, actor, detail, trace_id) VALUES (?, ?, ?, ?, ?, ?)",
                (
                    *(
                        item[key]
                        for key in (
                            "timestamp_unix_ms",
                            "kind",
                            "session_id",
                            "actor",
                            "detail",
                            "trace_id",
                        )
                    ),
                ),
            )
        connection.commit()


def test_sessions_read_back_in_ledger_order(tmp_path: Path) -> None:
    from capsem_builder.gate.tools.doctor.host_ledger import read_host_events

    path = tmp_path / "host.db"
    write_ledger(
        path,
        [
            event("service_started", None, 1),
            event("session_created", "a", 2),
            event("session_created", "b", 3),
            event("session_stopped", "a", 4),
        ],
    )
    assert validate_events(read_host_events(path)) == 4
    sessions = recent_sessions(path, 5)
    assert [session["id"] for session in sessions] == ["b", "a"]
    assert sessions[0]["status"] == "running"
    assert sessions[1]["status"] == "stopped"

@pytest.mark.parametrize("field", ["detail"])
def test_sqlite_text_corruption_is_a_doctor_failure(tmp_path: Path, field: str) -> None:
    from capsem_builder.gate.tools.doctor.doctor_session_host_verify import _verify_host_ledger
    from capsem_builder.gate.tools.doctor.doctor_session_verify import Results

    path = tmp_path / "host.db"
    write_ledger(path, [event("session_created", "test", 1), event("session_stopped", "test", 2)])
    with closing(sqlite3.connect(path)) as connection:
        connection.execute(f"UPDATE host_events SET {field} = ? WHERE id = 1", ("not binary",))
        connection.commit()
    results = Results()
    _verify_host_ledger(results, "test", path)
    assert results.failed == [f"host event {field} must be binary data"]
    assert not results.success
