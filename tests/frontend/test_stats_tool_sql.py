from __future__ import annotations

import re
import sqlite3
from contextlib import closing
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SQL_SOURCE = ROOT / "frontend/src/lib/sql.ts"


def _sql_const(name: str) -> str:
    source = SQL_SOURCE.read_text(encoding="utf-8")
    match = re.search(
        rf"export const {name} = `(?P<body>.*?)`;",
        source,
        flags=re.DOTALL,
    )
    assert match, f"missing SQL constant {name}"
    body = match.group("body")
    predicate = re.search(
        r'export const TOOL_CALL_LEDGER_WHERE = "(?P<body>.*?)";',
        source,
        flags=re.DOTALL,
    )
    assert predicate, "missing TOOL_CALL_LEDGER_WHERE"
    return body.replace("${TOOL_CALL_LEDGER_WHERE}", predicate.group("body"))


def test_tools_minimal_sql_reads_agy_native_tools_from_minimal_tool_call_schema() -> None:
    with closing(sqlite3.connect(":memory:")) as conn:
        conn.row_factory = sqlite3.Row
        conn.executescript(
            """
            CREATE TABLE model_calls (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL,
                provider TEXT NOT NULL,
                model TEXT,
                path TEXT NOT NULL,
                duration_ms INTEGER NOT NULL DEFAULT 0,
                trace_id TEXT
            );
            CREATE TABLE tool_calls (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                event_id TEXT NOT NULL,
                model_call_id INTEGER NOT NULL,
                provider TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL DEFAULT 'observed',
                call_index INTEGER NOT NULL,
                call_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                arguments TEXT,
                origin TEXT NOT NULL DEFAULT 'native',
                mcp_call_id INTEGER,
                trace_id TEXT,
                credential_ref TEXT
            );
            INSERT INTO model_calls (
                timestamp, provider, model, path, duration_ms, trace_id
            ) VALUES
              ('2026-06-23T00:37:50Z', 'google', 'gemini-3-flash-a',
               '/v1internal:streamGenerateContent', 2000, 'trace-agy'),
              ('2026-06-23T00:37:53Z', 'google', 'gemini-3-flash-a',
               '/v1internal:streamGenerateContent', 931, 'trace-agy');
            INSERT INTO tool_calls (
                event_id, model_call_id, provider, status, call_index, call_id,
                tool_name, arguments, origin, trace_id, credential_ref
            ) VALUES
              ('b7b1362d6c2f', 1, 'google', 'observed', 0, '6mehyzee',
               'list_dir', '{"DirectoryPath":"/root"}', 'native', 'trace-agy',
               'credential:blake3:6eac886fe25ebfb839812a97ef763f2ff25342eecbaf28642bcd4699d05f5d33'),
              ('635e6668985f', 2, 'google', 'observed', 0, 'hkuiu2so',
               'write_to_file', '{"TargetFile":"/root/agy.md"}', 'native', 'trace-agy',
               'credential:blake3:6eac886fe25ebfb839812a97ef763f2ff25342eecbaf28642bcd4699d05f5d33');
            """
        )

        rows = conn.execute(_sql_const("TOOLS_UNIFIED_MINIMAL_SQL")).fetchall()

        assert [row["tool_name"] for row in rows] == ["write_to_file", "list_dir"]
        assert [row["source"] for row in rows] == ["native", "native"]
        assert rows[0]["server_name"] == "model"
        assert rows[0]["decision"] == "allowed"
        assert rows[0]["duration_ms"] == 931
        assert rows[0]["arguments"] == '{"TargetFile":"/root/agy.md"}'
