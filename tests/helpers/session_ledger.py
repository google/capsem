"""Open a session ledger the way a black-box test reads it.

A repeated rule match stores its rule snapshot once, in `security_rule_runs`,
and each occurrence row in `security_rule_events` points at it through
`run_id` with its own `rule_json` NULL (see `capsem-logger`'s schema). That is
a storage decision, not a different event: each occurrence still matched
exactly one rule. Tests assert the event, so the connection this opens shows a
temporary `security_rule_events` with the rule resolved through its run and
the event's own columns, and nothing a test writes can reach the ledger.
"""

from __future__ import annotations

import sqlite3
from pathlib import Path

#: The occurrence columns a test reads; `run_id` is resolved, not shown.
_EVENT_COLUMNS = (
    "id",
    "timestamp_unix_ms",
    "event_id",
    "event_type",
    "rule_id",
    "rule_action",
    "detection_level",
    "trace_id",
    "turn_id",
    "credential_ref",
)


def resolve_rule_runs(conn: sqlite3.Connection) -> sqlite3.Connection:
    """Shadow `security_rule_events` with its rule-resolved logical view.

    A ledger without the table (another database) is returned untouched. A
    ledger with occurrences but no runs table is a broken schema and raises.
    """
    tables = {row[0] for row in conn.execute("SELECT name FROM main.sqlite_master WHERE type = 'table'")}
    if "security_rule_events" not in tables:
        return conn
    if "security_rule_runs" not in tables:
        raise AssertionError("session ledger has security_rule_events but no security_rule_runs")
    columns = ", ".join(f"e.{name}" for name in _EVENT_COLUMNS)
    conn.execute(
        f"""
        CREATE TEMP VIEW IF NOT EXISTS security_rule_events AS
        SELECT {columns}, COALESCE(e.rule_json, r.rule_json) AS rule_json
        FROM main.security_rule_events AS e
        LEFT JOIN main.security_rule_runs AS r ON r.id = e.run_id
        """
    )
    return conn


def open_session_ledger(db_path: Path | str) -> sqlite3.Connection:
    """Read-only connection to a session ledger, rule matches resolved."""
    return resolve_rule_runs(sqlite3.connect(f"file:{db_path}?mode=ro", uri=True))
