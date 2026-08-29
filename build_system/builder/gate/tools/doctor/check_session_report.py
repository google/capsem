"""Render integrity and event summaries for a single session database."""

from __future__ import annotations

import sqlite3
from pathlib import Path

SESSION_TABLES = {
    "net_events": [
        "id",
        "timestamp",
        "domain",
        "decision",
        "method",
        "path",
        "status_code",
        "duration_ms",
    ],
    "model_calls": [
        "id",
        "timestamp",
        "provider",
        "model",
        "input_tokens",
        "output_tokens",
        "stop_reason",
        "estimated_cost_usd",
        "duration_ms",
    ],
    "tool_calls": ["id", "model_call_id", "tool_name", "call_id", "origin"],
    "tool_responses": ["id", "model_call_id", "call_id", "is_error"],
    "fs_events": ["id", "timestamp", "action", "path", "size"],
}

BOLD = "\033[1m"
DIM = "\033[2m"
CYAN = "\033[36m"
GREEN = "\033[32m"
YELLOW = "\033[33m"
RED = "\033[31m"
RESET = "\033[0m"


def table(headers: list[str], rows: list[list], color: str = DIM) -> str:
    """Render a simple aligned table."""
    if not rows:
        return f"  {DIM}(empty){RESET}\n"
    widths = [len(header) for header in headers]
    string_rows = []
    for row in rows:
        cells = [str(value) if value is not None else "" for value in row]
        string_rows.append(cells)
        for index, cell in enumerate(cells):
            if index < len(widths):
                widths[index] = max(widths[index], len(cell))
    separator = "  ".join("-" * width for width in widths)
    header = "  ".join(value.ljust(width) for value, width in zip(headers, widths, strict=False))
    lines = [f"  {BOLD}{header}{RESET}", f"  {DIM}{separator}{RESET}"]
    for cells in string_rows:
        line = "  ".join(cell.ljust(width) for cell, width in zip(cells, widths, strict=False))
        lines.append(f"  {color}{line}{RESET}")
    return "\n".join(lines) + "\n"


def _event_counts(conn: sqlite3.Connection, existing: set[str]) -> None:
    print(f"{BOLD}Event counts:{RESET}")
    rows = [
        [name, str(conn.execute(f"SELECT COUNT(*) FROM {name}").fetchone()[0])]
        if name in existing
        else [name, "MISSING"]
        for name in SESSION_TABLES
    ]
    print(table(["table", "rows"], rows))


def _cross_checks(conn: sqlite3.Connection, existing: set[str]) -> None:
    if {"tool_calls", "tool_responses"} <= existing:
        orphans = conn.execute(
            "SELECT COUNT(*) FROM tool_calls tc"
            " LEFT JOIN tool_responses tr ON tc.call_id = tr.call_id"
            " WHERE tr.id IS NULL"
        ).fetchone()[0]
        total = conn.execute("SELECT COUNT(*) FROM tool_calls").fetchone()[0]
        if orphans > 0:
            print(f"  {YELLOW}tool_calls without responses: {orphans}/{total}{RESET}\n")
        elif total > 0:
            print(f"  {GREEN}All {total} tool_calls have matching responses{RESET}\n")

    if {"net_events", "model_calls"} <= existing:
        network_calls = conn.execute(
            "SELECT COUNT(*) FROM net_events"
            " WHERE domain LIKE '%.googleapis.com'"
            " OR domain LIKE '%.anthropic.com'"
            " OR domain LIKE '%.openai.com'"
        ).fetchone()[0]
        model_calls = conn.execute("SELECT COUNT(*) FROM model_calls").fetchone()[0]
        if network_calls > 0 and model_calls == 0:
            print(
                f"  {RED}Found {network_calls} AI-provider net_events but 0 model_calls"
                f" -- stream parsing may have failed{RESET}\n"
            )
        elif model_calls > 0:
            print(
                f"  {GREEN}{model_calls} model_calls from {network_calls}"
                f" AI-provider net_events{RESET}\n"
            )


def _model_quality(conn: sqlite3.Connection, existing: set[str]) -> None:
    if "model_calls" not in existing:
        return
    total = conn.execute("SELECT COUNT(*) FROM model_calls").fetchone()[0]
    if total == 0:
        return
    missing = {
        "NULL model": conn.execute(
            "SELECT COUNT(*) FROM model_calls WHERE model IS NULL"
        ).fetchone()[0],
        "NULL tokens": conn.execute(
            "SELECT COUNT(*) FROM model_calls WHERE input_tokens IS NULL AND output_tokens IS NULL"
        ).fetchone()[0],
        "NULL request_body_preview": conn.execute(
            "SELECT COUNT(*) FROM model_calls WHERE request_body_preview IS NULL"
        ).fetchone()[0],
    }
    warnings = [f"{label}: {count}/{total}" for label, count in missing.items() if count]
    if warnings:
        print(f"  {YELLOW}Data quality warnings:{RESET}")
        for warning in warnings:
            print(f"    {YELLOW}{warning}{RESET}")
        print()
    else:
        print(
            f"  {GREEN}All {total} model_calls have model, tokens, and preview populated{RESET}\n"
        )


def _tool_usage(conn: sqlite3.Connection, existing: set[str]) -> None:
    if "tool_calls" not in existing:
        return
    total = conn.execute("SELECT COUNT(*) FROM tool_calls").fetchone()[0]
    if total > 0:
        columns = {row[1] for row in conn.execute("PRAGMA table_info(tool_calls)")}
        if "origin" in columns:
            origins = conn.execute(
                "SELECT origin, COUNT(*) FROM tool_calls GROUP BY origin"
            ).fetchall()
            parts = [f"{row[1]} {row[0]}" for row in origins]
            print(f"  {CYAN}Tool origins: {', '.join(parts)} ({total} total){RESET}")
        print()
    rows = conn.execute(
        "SELECT tool_name, origin, decision, COUNT(*) as cnt,"
        " ROUND(AVG(duration_ms), 1) as avg_ms FROM tool_calls"
        " WHERE tool_name IS NOT NULL GROUP BY tool_name, origin, decision"
        " ORDER BY cnt DESC"
    ).fetchall()
    if rows:
        print(f"{BOLD}Tool usage:{RESET}")
        print(
            table(
                ["tool_name", "origin", "decision", "count", "avg_ms"],
                [[row[0], row[1], row[2], str(row[3]), str(row[4])] for row in rows],
            )
        )


def _previews(conn: sqlite3.Connection, existing: set[str], limit: int) -> None:
    for name, configured_columns in SESSION_TABLES.items():
        if name not in existing:
            continue
        rows = conn.execute(f"SELECT * FROM {name} ORDER BY id DESC LIMIT ?", (limit,)).fetchall()
        total = conn.execute(f"SELECT COUNT(*) FROM {name}").fetchone()[0]
        print(f"{BOLD}{name}{RESET} ({total} total, showing last {limit}):")
        if not rows:
            print(f"  {DIM}(empty){RESET}\n")
            continue
        columns = [column for column in configured_columns if column in dict(rows[0])]
        preview = []
        for row in rows:
            values = []
            for column in columns:
                value = dict(row).get(column)
                if isinstance(value, str) and len(value) > 60:
                    value = value[:57] + "..."
                values.append(value)
            preview.append(values)
        print(table(columns, preview))


def check_session(db_path: Path, preview_rows: int = 5) -> None:
    """Run all checks on a session DB and print results."""
    conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    conn.row_factory = sqlite3.Row
    try:
        print(f"\n{BOLD}{CYAN}Session: {db_path.parent.name}{RESET}")
        print(f"  {DIM}{db_path}{RESET}\n")
        existing = {
            row[0]
            for row in conn.execute("SELECT name FROM sqlite_master WHERE type='table'").fetchall()
        }
        missing = set(SESSION_TABLES) - existing
        if missing:
            print(f"  {RED}Missing tables: {', '.join(sorted(missing))}{RESET}\n")
        else:
            print(f"  {GREEN}All expected tables present{RESET}\n")
        _event_counts(conn, existing)
        _cross_checks(conn, existing)
        _model_quality(conn, existing)
        _tool_usage(conn, existing)
        _previews(conn, existing, preview_rows)
    finally:
        conn.close()
