"""Read the host ledger: what the service did to every session.

The service records each session's creation and stop, and its own start and
stop, as hash-chained rows in `sessions/host.db`. These helpers read it
read-only and recompute the chain exactly as `capsem_logger::chain_hash`
does, so a doctor run can say whether the record was edited.
"""

from __future__ import annotations

import sqlite3
import struct
from contextlib import closing
from pathlib import Path
from typing import Any

import blake3

GENESIS_HASH = bytes(32)
SESSION_CREATED = "session_created"
SESSION_STOPPED = "session_stopped"


def host_ledger_path(capsem_home: Path) -> Path:
    return capsem_home / "sessions" / "host.db"


def read_host_events(path: Path) -> list[dict[str, Any]]:
    """Every host event in ledger order, with its stored chain link."""
    with closing(sqlite3.connect(f"file:{path}?mode=ro", uri=True)) as connection:
        connection.row_factory = sqlite3.Row
        rows = connection.execute(
            "SELECT timestamp_unix_ms, kind, session_id, actor, detail, trace_id, prev_hash, hash"
            " FROM host_events ORDER BY id"
        ).fetchall()
    return [dict(row) for row in rows]


def chain_hash(previous: bytes, event: dict[str, Any]) -> bytes:
    """The Rust `chain_hash`: every field, length-prefixed in a fixed order."""
    hasher = blake3.blake3(previous)
    hasher.update(struct.pack("<q", event["timestamp_unix_ms"]))
    for value in (
        event["kind"].encode(),
        None if event["session_id"] is None else event["session_id"].encode(),
        event["actor"].encode(),
        bytes(event["detail"]),
        None if event["trace_id"] is None else event["trace_id"].encode(),
    ):
        if value is None:
            hasher.update(b"\x00")
        else:
            hasher.update(b"\x01" + struct.pack("<Q", len(value)) + value)
    return hasher.digest()


def verify_chain(events: list[dict[str, Any]]) -> int:
    """How many events the chain covers; `ValueError` naming the first break."""
    previous = GENESIS_HASH
    for index, event in enumerate(events):
        if bytes(event["prev_hash"]) != previous or bytes(event["hash"]) != chain_hash(
            previous, event
        ):
            raise ValueError(f"host event chain is broken at event {index}")
        previous = bytes(event["hash"])
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
