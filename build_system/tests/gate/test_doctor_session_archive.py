"""The session doctor checks the body archive against the index into it."""

from __future__ import annotations

import shutil
import sqlite3
import sys
from pathlib import Path

import pytest
from capsem_builder.gate.tools.doctor import check_session, check_session_archive

REPOSITORY_ROOT = Path(__file__).resolve().parents[3]
FIXTURE = REPOSITORY_ROOT / "tests" / "fixtures" / "session"


@pytest.fixture
def ledger(tmp_path: Path) -> Path:
    """A writable copy of the fixture session: its ledger and its archive."""
    shutil.copy(FIXTURE / "test.db", tmp_path / "session.db")
    shutil.copy(FIXTURE / "test.bodies", tmp_path / "session.bodies")
    return tmp_path / "session.db"


def _findings(db: Path, *, verify: bool = False) -> check_session_archive.ArchiveFindings:
    conn = sqlite3.connect(db)
    try:
        return check_session_archive.check_body_archive(conn, db, verify_bodies=verify)
    finally:
        conn.close()


def _edit(db: Path, sql: str) -> None:
    """Apply a damage the writer would never make; foreign keys stay off."""
    conn = sqlite3.connect(db)
    try:
        conn.execute("PRAGMA foreign_keys = OFF")
        conn.execute(sql)
        conn.commit()
    finally:
        conn.close()


def test_an_intact_ledger_agrees_with_its_archive_body_for_body(ledger: Path) -> None:
    found = _findings(ledger, verify=True)
    assert found.problems == []
    assert found.bodies > 0
    assert found.verified == found.bodies
    assert found.archive_bytes == found.blocks_end


def test_a_truncated_archive_is_reported_against_the_blocks_it_lost(ledger: Path) -> None:
    archive = ledger.with_suffix(".bodies")
    archive.write_bytes(archive.read_bytes()[:-1])
    found = _findings(ledger)
    assert len(found.problems) == 1
    assert "last recorded block ends at" in found.problems[0]


def test_a_body_that_runs_past_its_block_is_reported(ledger: Path) -> None:
    _edit(ledger, "UPDATE event_body_blobs SET body_offset = 1000000000 WHERE id = 1")
    assert _findings(ledger).problems == ["1 bodies run past the end of their block's raw bytes"]


def test_a_body_naming_an_unrecorded_block_is_reported(ledger: Path) -> None:
    _edit(ledger, "UPDATE event_body_blobs SET block_offset = 7 WHERE id = 1")
    assert _findings(ledger).problems == ["1 bodies name a block body_blocks does not record"]


def test_a_body_whose_bytes_changed_fails_the_end_to_end_read(ledger: Path) -> None:
    _edit(
        ledger,
        "UPDATE event_body_blobs SET body_hash = 'blake3:' || printf('%064d', 0) WHERE id = 1",
    )
    found = _findings(ledger, verify=True)
    assert len(found.problems) == 1
    assert "does not match the row that named it" in found.problems[0]
    assert found.verified == found.bodies - 1


def test_overflow_windows_are_counted_with_the_changes_they_deferred(ledger: Path) -> None:
    _edit(
        ledger,
        "INSERT INTO fs_events (timestamp, action, path, size) VALUES ('t', 'overflow', '', 41)",
    )
    conn = sqlite3.connect(ledger)
    try:
        assert check_session_archive.fs_overflow(conn) == (1, 41)
    finally:
        conn.close()


def test_the_command_exits_nonzero_on_a_broken_archive(
    ledger: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.setattr(sys, "argv", ["check_session.py", "--db", str(ledger)])
    assert check_session.main() == 0
    ledger.with_suffix(".bodies").write_bytes(b"")
    assert check_session.main() == 1
    assert "last recorded block ends at" in capsys.readouterr().out
