"""The black-box ledger view resolves a rule match through its run."""

from __future__ import annotations

import re
import sqlite3
from contextlib import closing
from pathlib import Path

import pytest
from helpers.session_ledger import COUNTED_TOOL_ORIGINS, open_session_ledger

ROOT = Path(__file__).resolve().parents[1]


def _ledger(tmp_path: Path) -> Path:
    db = tmp_path / "session.db"
    # `with sqlite3.connect(...)` commits but does not close; `closing` does.
    with closing(sqlite3.connect(db)) as conn:
        conn.executescript(
            """
            CREATE TABLE security_rule_runs (id INTEGER PRIMARY KEY, rule_json TEXT NOT NULL);
            CREATE TABLE security_rule_events (
                id INTEGER PRIMARY KEY, timestamp_unix_ms INTEGER, event_id TEXT, event_type TEXT,
                rule_id TEXT, rule_action TEXT, detection_level TEXT, rule_json TEXT,
                run_id INTEGER, trace_id TEXT, turn_id TEXT, credential_ref TEXT);
            INSERT INTO security_rule_runs VALUES (1, '{"from":"run"}');
            INSERT INTO security_rule_events (id, event_id, rule_json, run_id)
                VALUES (1, 'aaaaaaaaaaaa', NULL, 1), (2, 'bbbbbbbbbbbb', '{"from":"row"}', NULL);
            """
        )
    return db


def test_a_repeated_match_reads_its_rule_from_the_run(tmp_path: Path) -> None:
    with closing(open_session_ledger(_ledger(tmp_path))) as conn:
        rows = conn.execute("SELECT event_id, rule_json FROM security_rule_events ORDER BY id").fetchall()
        columns = {row[1] for row in conn.execute("PRAGMA table_info(security_rule_events)")}
    assert rows == [("aaaaaaaaaaaa", '{"from":"run"}'), ("bbbbbbbbbbbb", '{"from":"row"}')]
    assert "run_id" not in columns and "rule_json" in columns


def test_the_view_is_read_only(tmp_path: Path) -> None:
    with closing(open_session_ledger(_ledger(tmp_path))) as conn, pytest.raises(sqlite3.OperationalError):
        conn.execute("INSERT INTO main.security_rule_runs VALUES (2, '{}')")


def test_a_failed_open_leaves_no_connection_behind(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """A ledger caught half-created raises, and the connection is closed."""
    db = tmp_path / "session.db"
    with closing(sqlite3.connect(db)) as conn:
        conn.execute("CREATE TABLE security_rule_events (id INTEGER PRIMARY KEY)")
    opened: list[sqlite3.Connection] = []
    real_connect = sqlite3.connect

    def spy(*args, **kwargs):
        connection = real_connect(*args, **kwargs)
        opened.append(connection)
        return connection

    monkeypatch.setattr(sqlite3, "connect", spy)
    with pytest.raises(AssertionError, match="no security_rule_runs"):
        open_session_ledger(db)
    monkeypatch.undo()
    (connection,) = opened
    with pytest.raises(sqlite3.ProgrammingError):
        connection.execute("SELECT 1")


def test_ironbank_opens_ledgers_only_through_the_helper() -> None:
    """A raw read-only connect would see NULL rule_json on every repeated match."""
    raw = [
        f"{path.relative_to(ROOT)}:{number}"
        for directory in ("tests/ironbank", "tests/capsem-e2e")
        for path in sorted((ROOT / directory).rglob("*.py"))
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1)
        if "?mode=ro" in line and "sqlite3.connect" in line
    ]
    assert not raw, "open session ledgers with helpers.session_ledger.open_session_ledger:\n" + "\n".join(raw)


def test_counted_tool_origins_match_the_logger() -> None:
    """The oracle counts what the counters count, or it proves nothing."""
    source = (ROOT / "crates/capsem-logger/src/counters.rs").read_text(encoding="utf-8")
    declared = re.search(r"pub const COUNTED_TOOL_ORIGINS: \[&str; \d+\] = \[([^\]]*)\];", source)
    assert declared is not None, "COUNTED_TOOL_ORIGINS moved; point this test at it"
    assert tuple(re.findall(r'"([^"]+)"', declared.group(1))) == COUNTED_TOOL_ORIGINS
