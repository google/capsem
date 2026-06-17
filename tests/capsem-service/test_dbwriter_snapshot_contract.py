"""Service-side DbWriter and snapshot-route contract.

Snapshots are host recovery state. They must stay route/file backed and must
not become user-facing session.db activity.
"""

from __future__ import annotations

import json
import platform
import sqlite3
from pathlib import Path
from typing import Any

import tomllib

from helpers.service import ServiceInstance, materialize_test_profiles


ROOT = Path(__file__).resolve().parents[2]


def _profile_contract(tmp_dir: Path) -> dict[str, Any]:
    profiles_dir = materialize_test_profiles(tmp_dir)
    profile = tomllib.loads((profiles_dir / "code" / "profile.toml").read_text())
    arch = "arm64" if platform.machine() == "arm64" else "x86_64"
    assets = profile["assets"]["arch"][arch]
    return {
        "revision": profile["revision"],
        "pins": {
            "kernel": {
                "name": assets["kernel"]["name"],
                "hash": assets["kernel"]["hash"],
            },
            "initrd": {
                "name": assets["initrd"]["name"],
                "hash": assets["initrd"]["hash"],
            },
            "rootfs": {
                "name": assets["rootfs"]["name"],
                "hash": assets["rootfs"]["hash"],
            },
        },
    }


def _write_persistent_registry(tmp_dir: Path, session_id: str, session_dir: Path) -> None:
    contract = _profile_contract(tmp_dir)
    entry = {
        "name": session_id,
        "profile_id": "code",
        "profile_revision": contract["revision"],
        "profile_payload_hash": "blake3:" + ("0" * 64),
        "asset_pins": contract["pins"],
        "ram_mb": 2048,
        "cpus": 2,
        "base_version": "0.0.0-test",
        "created_at": "2026-06-16T00:00:00Z",
        "session_dir": str(session_dir),
        "defunct": False,
    }
    (tmp_dir / "persistent_registry.json").write_text(
        json.dumps({"vms": {session_id: entry}}, indent=2),
        encoding="utf-8",
    )


def _write_snapshot_metadata(session_dir: Path) -> None:
    snapshots = session_dir / "auto_snapshots"
    for slot, origin, name, millis in [
        (0, "auto", None, 1_789_000_000_000),
        (10, "manual", "manual_check", 1_789_000_001_000),
    ]:
        slot_dir = snapshots / str(slot)
        (slot_dir / "workspace").mkdir(parents=True, exist_ok=True)
        (slot_dir / "system").mkdir(parents=True, exist_ok=True)
        (slot_dir / "metadata.json").write_text(
            json.dumps(
                {
                    "slot": slot,
                    "timestamp": "2026-06-16T00:00:00Z",
                    "epoch_secs": millis // 1000,
                    "epoch_millis": millis,
                    "origin": origin,
                    "name": name,
                    "hash": "blake3:" + ("a" * 64) if origin == "manual" else None,
                }
            ),
            encoding="utf-8",
        )


def _write_toxic_session_db(session_dir: Path) -> None:
    conn = sqlite3.connect(session_dir / "session.db")
    try:
        conn.execute(
            "CREATE TABLE snapshot_events (id INTEGER PRIMARY KEY, event TEXT NOT NULL)"
        )
        conn.execute("INSERT INTO snapshot_events (event) VALUES ('must-not-leak')")
        conn.execute(
            "CREATE TABLE fs_events (id INTEGER PRIMARY KEY, path TEXT NOT NULL)"
        )
        conn.execute("INSERT INTO fs_events (path) VALUES ('snapshot-leak-marker')")
        conn.commit()
    finally:
        conn.close()


def test_snapshot_routes_are_file_backed_and_ignore_session_db() -> None:
    service = ServiceInstance()
    session_id = "code-snapshot-contract"
    session_dir = service.tmp_dir / "persistent" / session_id
    session_dir.mkdir(parents=True)
    (session_dir / "workspace").mkdir()
    (session_dir / "system").mkdir()
    _write_snapshot_metadata(session_dir)
    _write_toxic_session_db(session_dir)
    _write_persistent_registry(service.tmp_dir, session_id, session_dir)

    try:
        service.start()
        client = service.client()

        status = client.get(f"/vms/{session_id}/snapshots/status")
        assert set(status) == {
            "total",
            "auto_count",
            "manual_count",
            "manual_available",
            "snapshots",
        }
        assert status["total"] == 2
        assert status["auto_count"] == 1
        assert status["manual_count"] == 1
        assert status["manual_available"] == 11
        assert [snapshot["origin"] for snapshot in status["snapshots"]] == [
            "manual",
            "auto",
        ]
        assert status["snapshots"][0]["checkpoint"] == "cp-10"
        assert status["snapshots"][0]["name"] == "manual_check"
        assert "must-not-leak" not in json.dumps(status)
        assert "snapshot-leak-marker" not in json.dumps(status)

        listing = client.get(f"/vms/{session_id}/snapshots/list")
        assert listing == {
            "total": status["total"],
            "snapshots": status["snapshots"],
        }
    finally:
        service.stop()


def test_dbwriter_and_snapshot_source_boundaries_are_single_rail() -> None:
    service_main = (ROOT / "crates/capsem-service/src/main.rs").read_text()
    service_prod = service_main.split("\n#[cfg(test)]\nmod tests;", 1)[0]
    process_main = (ROOT / "crates/capsem-process/src/main.rs").read_text()
    process_prod = process_main.split("\n#[cfg(test)]\nmod tests", 1)[0]
    process_vsock = (ROOT / "crates/capsem-process/src/vsock.rs").read_text()
    logger_schema = (ROOT / "crates/capsem-logger/src/schema.rs").read_text()
    logger_writer = (ROOT / "crates/capsem-logger/src/writer.rs").read_text()

    assert 'DbWriter::open(&resolve_session_dir(&state' not in service_prod
    assert 'DbWriter::open(&session_dir.join("session.db")' not in service_prod
    assert "DbWriter::open(&state.main_db_path()" in service_prod
    assert 'session_dir.join("session.db")' in service_prod
    assert "snapshot_status_from_session_dir(&session_dir)" in service_prod
    assert "send_ipc_command(" in service_prod
    assert "ServiceToProcess::SnapshotStatus" in service_prod

    assert "capsem_logger::DbWriter::open(" in process_prod
    assert '&session_dir.join("session.db")' in process_prod
    assert "Arc<capsem_logger::DbWriter>" in process_vsock
    assert "rusqlite::Connection" not in process_vsock
    assert "write_many" not in process_vsock

    assert "DROP TABLE IF EXISTS snapshot_events" in logger_schema
    assert "snapshot.event must not be a security-event type" in logger_schema
    assert "pub struct DbWriter" in logger_writer
    assert "tokio::sync::mpsc::channel(capacity)" in logger_writer
    assert '.name("capsem-db-writer".into())' in logger_writer
