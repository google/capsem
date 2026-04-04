"""Verify session.db exists after boot with all expected tables."""

import pytest

pytestmark = pytest.mark.session_lifecycle

EXPECTED_TABLES = [
    "net_events",
    "model_calls",
    "tool_calls",
    "tool_responses",
    "mcp_calls",
    "fs_events",
    "snapshot_events",
]


def test_db_exists_after_boot(lifecycle_env):
    """session.db file exists in the session directory."""
    _, vm_name, tmp_dir, _ = lifecycle_env
    db_path = tmp_dir / "sessions" / vm_name / "session.db"
    assert db_path.exists(), f"session.db not found at {db_path}"


def test_all_tables_present(lifecycle_db):
    """session.db has all 7 expected tables."""
    tables = [
        r[0] for r in lifecycle_db.execute(
            "SELECT name FROM sqlite_master WHERE type='table'"
        ).fetchall()
    ]
    for table in EXPECTED_TABLES:
        assert table in tables, f"Missing table: {table} (found: {tables})"
