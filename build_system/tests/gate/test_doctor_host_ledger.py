"""The doctor tools read the host ledger the way the service writes it."""

from __future__ import annotations

import sqlite3
from contextlib import closing
from pathlib import Path

import pytest
from capsem_builder.gate.tools.doctor.host_ledger import (
    GENESIS_HASH,
    chain_hash,
    recent_sessions,
    verify_chain,
)

# Pinned by crates/capsem-logger/src/events/host/tests.rs too.
GOLDEN = "334ad1daa78a4731b89861fb2ad5fc4cee7cc3fa98eb2669542245d40c90a089"


def event(kind: str, session: str | None, at: int) -> dict:
    return {
        "timestamp_unix_ms": at,
        "kind": kind,
        "session_id": session,
        "actor": "service",
        "detail": b"\x80",
        "trace_id": None,
    }


def test_the_python_chain_hash_matches_the_rust_golden_vector() -> None:
    golden = event("session_stopped", "golden", 1_791_000_000_000)
    golden["detail"] = bytes([0x81, 0xA1, 0x61, 0x05])
    assert chain_hash(GENESIS_HASH, golden).hex() == GOLDEN


def write_ledger(path: Path, events: list[dict]) -> None:
    with closing(sqlite3.connect(path)) as connection:
        connection.execute(
            "CREATE TABLE host_events (id INTEGER PRIMARY KEY, timestamp_unix_ms INTEGER, kind TEXT,"
            " session_id TEXT, actor TEXT, detail BLOB, trace_id TEXT, prev_hash BLOB, hash BLOB)"
        )
        previous = GENESIS_HASH
        for item in events:
            digest = chain_hash(previous, item)
            connection.execute(
                "INSERT INTO host_events (timestamp_unix_ms, kind, session_id, actor, detail, trace_id,"
                " prev_hash, hash) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
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
                    previous,
                    digest,
                ),
            )
            previous = digest
        connection.commit()


def test_sessions_and_chain_read_back(tmp_path: Path) -> None:
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
    assert verify_chain(read_host_events(path)) == 4
    sessions = recent_sessions(path, 5)
    assert [session["id"] for session in sessions] == ["b", "a"]
    assert sessions[0]["status"] == "running"
    assert sessions[1]["status"] == "stopped"

    with closing(sqlite3.connect(path)) as connection:
        connection.execute("UPDATE host_events SET actor = 'someone-else' WHERE id = 2")
        connection.commit()
    with pytest.raises(ValueError, match="broken at event 1"):
        verify_chain(read_host_events(path))
