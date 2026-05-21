#!/usr/bin/env python3
"""Check session DB integrity and show a summary of recorded events."""

import argparse
import gzip
import json
import os
import sqlite3
import sys
import tempfile
from pathlib import Path
from typing import Optional

CAPSEM_HOME = Path(os.environ.get("CAPSEM_HOME", Path.home() / ".capsem"))
RUN_DIR = Path(os.environ.get("CAPSEM_RUN_DIR", CAPSEM_HOME / "run"))
SESSIONS_DIR = RUN_DIR / "sessions"
MAIN_DB = CAPSEM_HOME / "sessions" / "main.db"

# Tables expected in current session.db files with their key columns for
# preview. Older DBs may only have the core six tables; check_session keeps
# those readable while calling out current-version coverage gaps separately.
SESSION_TABLES = {
    "net_events": [
        "id", "timestamp", "domain", "decision", "method", "path",
        "status_code", "duration_ms", "policy_mode", "policy_action",
        "policy_rule", "policy_reason", "trace_id",
    ],
    "dns_events": [
        "id", "timestamp", "qname", "rcode", "decision", "matched_rule",
        "policy_mode", "policy_action", "policy_rule", "policy_reason",
        "trace_id",
    ],
    "model_calls": [
        "id", "timestamp", "provider", "model", "input_tokens",
        "output_tokens", "stop_reason", "estimated_cost_usd", "duration_ms",
        "trace_id",
    ],
    "tool_calls": [
        "id", "model_call_id", "tool_name", "call_id", "origin",
        "mcp_call_id", "trace_id",
    ],
    "tool_responses": [
        "id", "model_call_id", "call_id", "is_error", "trace_id",
    ],
    "mcp_calls": [
        "id", "timestamp", "server_name", "method", "tool_name", "decision",
        "duration_ms", "policy_mode", "policy_action", "policy_rule",
        "policy_reason", "trace_id",
    ],
    "exec_events": [
        "id", "timestamp", "exec_id", "command", "exit_code", "duration_ms",
        "source", "mcp_call_id", "trace_id",
    ],
    "fs_events": [
        "id", "timestamp", "action", "path", "size", "trace_id",
    ],
    "snapshot_events": [
        "id", "timestamp", "slot", "origin", "name", "files_count",
        "trace_id",
    ],
    "audit_events": [
        "id", "timestamp", "pid", "ppid", "uid", "exe", "comm",
        "exit_code", "audit_id", "exec_event_id", "trace_id",
    ],
}

CORE_REQUIRED_TABLES = {
    "net_events",
    "model_calls",
    "tool_calls",
    "tool_responses",
    "mcp_calls",
    "fs_events",
}

CURRENT_VERSION_TABLES = set(SESSION_TABLES) - CORE_REQUIRED_TABLES

POLICY_V2_COLUMNS = {
    "net_events": ["policy_mode", "policy_action", "policy_rule", "policy_reason", "trace_id"],
    "dns_events": ["policy_mode", "policy_action", "policy_rule", "policy_reason", "trace_id"],
    "mcp_calls": ["policy_mode", "policy_action", "policy_rule", "policy_reason", "trace_id"],
}

BOLD = "\033[1m"
DIM = "\033[2m"
BLUE = "\033[34m"
PURPLE = "\033[35m"
CYAN = "\033[36m"
GREEN = "\033[32m"
YELLOW = "\033[33m"
RED = "\033[31m"
RESET = "\033[0m"


def table(headers: list[str], rows: list[list], color: str = DIM) -> str:
    """Render a simple aligned table."""
    if not rows:
        return f"  {DIM}(empty){RESET}\n"
    widths = [len(h) for h in headers]
    str_rows = []
    for row in rows:
        cells = [str(v) if v is not None else "" for v in row]
        str_rows.append(cells)
        for i, c in enumerate(cells):
            if i < len(widths):
                widths[i] = max(widths[i], len(c))
    sep = "  ".join("-" * w for w in widths)
    hdr = "  ".join(h.ljust(w) for h, w in zip(headers, widths))
    lines = [f"  {BOLD}{hdr}{RESET}", f"  {DIM}{sep}{RESET}"]
    for cells in str_rows:
        line = "  ".join(c.ljust(w) for c, w in zip(cells, widths))
        lines.append(f"  {color}{line}{RESET}")
    return "\n".join(lines) + "\n"


def list_recent_sessions(n: int = 5) -> list[dict]:
    """Return the N most recent sessions from main.db."""
    if not MAIN_DB.exists():
        print(f"{RED}main.db not found at {MAIN_DB}{RESET}", file=sys.stderr)
        sys.exit(1)
    conn = sqlite3.connect(f"file:{MAIN_DB}?mode=ro", uri=True)
    conn.row_factory = sqlite3.Row
    rows = conn.execute(
        "SELECT id, mode, status, created_at, stopped_at,"
        " total_requests, allowed_requests, denied_requests,"
        " total_input_tokens, total_output_tokens,"
        " total_estimated_cost, total_tool_calls, total_mcp_calls,"
        " total_file_events"
        " FROM sessions ORDER BY created_at DESC LIMIT ?",
        (n,),
    ).fetchall()
    conn.close()
    return [dict(r) for r in rows]


def table_columns(conn: sqlite3.Connection, table_name: str) -> set[str]:
    """Return column names for a table, or an empty set when absent."""
    return {r[1] for r in conn.execute(f"PRAGMA table_info({table_name})").fetchall()}


def resolve_session(session_id: Optional[str]) -> Path:
    """Resolve a session ID (or latest) to its session.db path.

    If the DB has been compressed (session.db.gz), decompress to a temp file.
    """
    if session_id:
        session_dir = SESSIONS_DIR / session_id
    else:
        sessions = list_recent_sessions(1)
        if not sessions:
            print(f"{RED}No sessions found in main.db{RESET}", file=sys.stderr)
            sys.exit(1)
        session_dir = SESSIONS_DIR / sessions[0]["id"]

    db = session_dir / "session.db"
    if db.exists():
        return db

    gz = session_dir / "session.db.gz"
    if gz.exists():
        # Decompress to a temp file.
        tmp = tempfile.NamedTemporaryFile(suffix=".db", delete=False)
        with gzip.open(gz, "rb") as f:
            tmp.write(f.read())
        tmp.close()
        print(f"  {DIM}(decompressed {gz.name} to temp file){RESET}")
        return Path(tmp.name)

    sid = session_dir.name
    print(
        f"{RED}session.db not found for {sid}{RESET}",
        file=sys.stderr,
    )
    sys.exit(1)


def check_session(db_path: Path, preview_rows: int = 5):
    """Run all checks on a session DB and print results."""
    session_id = db_path.parent.name
    conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    conn.row_factory = sqlite3.Row

    # -- Header --
    print(f"\n{BOLD}{CYAN}Session: {session_id}{RESET}")
    print(f"  {DIM}{db_path}{RESET}\n")

    # -- Table existence check --
    existing = {
        r[0]
        for r in conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table'"
        ).fetchall()
    }
    missing_required = CORE_REQUIRED_TABLES - existing
    missing_current = CURRENT_VERSION_TABLES - existing
    if missing_required:
        print(
            f"  {RED}Missing required tables: "
            f"{', '.join(sorted(missing_required))}{RESET}\n"
        )
    elif missing_current:
        print(
            f"  {YELLOW}Core tables present; optional/current tables absent "
            f"(old DB compatible): {', '.join(sorted(missing_current))}{RESET}\n"
        )
    else:
        print(f"  {GREEN}All current-version tables present{RESET}\n")

    policy_column_gaps: list[str] = []
    for tbl, cols in POLICY_V2_COLUMNS.items():
        if tbl not in existing:
            continue
        present = table_columns(conn, tbl)
        missing_cols = [c for c in cols if c not in present]
        if missing_cols:
            policy_column_gaps.append(f"{tbl}: {', '.join(missing_cols)}")
    if policy_column_gaps:
        print(f"  {YELLOW}Policy V2 columns unavailable (old DB compatible):{RESET}")
        for gap in policy_column_gaps:
            print(f"    {YELLOW}{gap}{RESET}")
        print()

    # -- Row counts --
    print(f"{BOLD}Event counts:{RESET}")
    count_headers = ["table", "rows"]
    count_rows = []
    for tbl in SESSION_TABLES:
        if tbl in existing:
            n = conn.execute(f"SELECT COUNT(*) FROM {tbl}").fetchone()[0]
            count_rows.append([tbl, str(n)])
        elif tbl in CURRENT_VERSION_TABLES:
            count_rows.append([tbl, "MISSING (old DB optional)"])
        else:
            count_rows.append([tbl, "MISSING"])
    print(table(count_headers, count_rows))

    # -- Cross-check: tool_calls without matching tool_responses --
    if "tool_calls" in existing and "tool_responses" in existing:
        orphans = conn.execute(
            "SELECT COUNT(*) FROM tool_calls tc"
            " LEFT JOIN tool_responses tr"
            "   ON tc.call_id = tr.call_id"
            " WHERE tr.id IS NULL"
        ).fetchone()[0]
        total_tc = conn.execute("SELECT COUNT(*) FROM tool_calls").fetchone()[0]
        if orphans > 0:
            print(
                f"  {YELLOW}tool_calls without responses:"
                f" {orphans}/{total_tc}{RESET}\n"
            )
        elif total_tc > 0:
            print(
                f"  {GREEN}All {total_tc} tool_calls have"
                f" matching responses{RESET}\n"
            )

    # -- Cross-check: net_events vs model_calls consistency --
    if "net_events" in existing and "model_calls" in existing:
        net_ai = conn.execute(
            "SELECT COUNT(*) FROM net_events"
            " WHERE domain LIKE '%.googleapis.com'"
            "    OR domain LIKE '%.anthropic.com'"
            "    OR domain LIKE '%.openai.com'"
        ).fetchone()[0]
        mc = conn.execute("SELECT COUNT(*) FROM model_calls").fetchone()[0]
        if net_ai > 0 and mc == 0:
            print(
                f"  {RED}Found {net_ai} AI-provider net_events but"
                f" 0 model_calls -- stream parsing may have failed{RESET}\n"
            )
        elif mc > 0:
            print(
                f"  {GREEN}{mc} model_calls from {net_ai}"
                f" AI-provider net_events{RESET}\n"
            )

    # -- Data quality warnings: model_calls with NULL critical fields --
    if "model_calls" in existing:
        mc_total = conn.execute("SELECT COUNT(*) FROM model_calls").fetchone()[0]
        if mc_total > 0:
            null_model = conn.execute(
                "SELECT COUNT(*) FROM model_calls WHERE model IS NULL"
            ).fetchone()[0]
            null_tokens = conn.execute(
                "SELECT COUNT(*) FROM model_calls"
                " WHERE input_tokens IS NULL AND output_tokens IS NULL"
            ).fetchone()[0]
            null_preview = conn.execute(
                "SELECT COUNT(*) FROM model_calls WHERE request_body_preview IS NULL"
            ).fetchone()[0]
            warnings = []
            if null_model > 0:
                warnings.append(f"NULL model: {null_model}/{mc_total}")
            if null_tokens > 0:
                warnings.append(f"NULL tokens: {null_tokens}/{mc_total}")
            if null_preview > 0:
                warnings.append(f"NULL request_body_preview: {null_preview}/{mc_total}")
            if warnings:
                print(f"  {YELLOW}Data quality warnings:{RESET}")
                for w in warnings:
                    print(f"    {YELLOW}{w}{RESET}")
                print()
            else:
                print(
                    f"  {GREEN}All {mc_total} model_calls have"
                    f" model, tokens, and preview populated{RESET}\n"
                )

    # -- Tool lifecycle: origin breakdown + mcp correlation --
    if "tool_calls" in existing:
        tc_total = conn.execute("SELECT COUNT(*) FROM tool_calls").fetchone()[0]
        if tc_total > 0:
            # Check if origin column exists (may be missing on old DBs)
            tc_cols = table_columns(conn, "tool_calls")
            if "origin" in tc_cols:
                origin_rows = conn.execute(
                    "SELECT origin, COUNT(*) FROM tool_calls GROUP BY origin"
                ).fetchall()
                parts = [f"{r[1]} {r[0]}" for r in origin_rows]
                print(
                    f"  {CYAN}Tool origins: {', '.join(parts)}"
                    f" ({tc_total} total){RESET}"
                )
            # Show matching mcp_calls per tool if both tables exist
            if "mcp_calls" in existing:
                mc_cols = table_columns(conn, "mcp_calls")
                model_cols = table_columns(conn, "model_calls")
                mcp_total = conn.execute(
                    "SELECT COUNT(*) FROM mcp_calls"
                ).fetchone()[0]
                if mcp_total > 0:
                    matched = 0
                    method = "none"
                    if "mcp_call_id" in tc_cols:
                        matched = conn.execute(
                            "SELECT COUNT(DISTINCT tc.id) FROM tool_calls tc"
                            " JOIN mcp_calls mc ON tc.mcp_call_id = mc.id"
                        ).fetchone()[0]
                        method = "exact mcp_call_id"
                    if (
                        matched == 0
                        and "trace_id" in tc_cols
                        and "trace_id" in mc_cols
                    ):
                        matched = conn.execute(
                            "SELECT COUNT(DISTINCT tc.id) FROM tool_calls tc"
                            " JOIN mcp_calls mc ON tc.trace_id = mc.trace_id"
                            " WHERE tc.trace_id IS NOT NULL"
                        ).fetchone()[0]
                        method = "trace_id fallback"
                    if (
                        matched == 0
                        and "timestamp" in model_cols
                        and "model_call_id" in tc_cols
                    ):
                        matched = conn.execute(
                            "SELECT COUNT(DISTINCT tc.id) FROM tool_calls tc"
                            " JOIN model_calls model ON tc.model_call_id = model.id"
                            " JOIN mcp_calls mc ON tc.tool_name = mc.tool_name"
                            " WHERE ABS(strftime('%s', mc.timestamp)"
                            "          - strftime('%s', model.timestamp)) <= 60"
                        ).fetchone()[0]
                        method = "trace+timestamp fallback"
                    print(
                        f"  {CYAN}Guest MCP calls: {mcp_total}"
                        f" ({matched} correlated with tool_calls via {method}){RESET}"
                    )
            print()

    # -- MCP tool usage breakdown --
    if "mcp_calls" in existing:
        mcp_total = conn.execute("SELECT COUNT(*) FROM mcp_calls").fetchone()[0]
        if mcp_total > 0:
            tool_rows = conn.execute(
                "SELECT tool_name, decision, COUNT(*) as cnt,"
                " ROUND(AVG(duration_ms), 1) as avg_ms"
                " FROM mcp_calls"
                " WHERE tool_name IS NOT NULL"
                " GROUP BY tool_name, decision"
                " ORDER BY cnt DESC"
            ).fetchall()
            if tool_rows:
                print(f"{BOLD}MCP tool usage:{RESET}")
                print(
                    table(
                        ["tool_name", "decision", "count", "avg_ms"],
                        [[r[0], r[1], str(r[2]), str(r[3])] for r in tool_rows],
                    )
                )

    # -- Preview rows per table --
    for tbl, cols in SESSION_TABLES.items():
        if tbl not in existing:
            continue
        rows = conn.execute(
            f"SELECT * FROM {tbl} ORDER BY id DESC LIMIT ?",
            (preview_rows,),
        ).fetchall()
        n = conn.execute(f"SELECT COUNT(*) FROM {tbl}").fetchone()[0]
        print(f"{BOLD}{tbl}{RESET} ({n} total, showing last {preview_rows}):")
        if not rows:
            print(f"  {DIM}(empty){RESET}\n")
            continue
        # Use only the configured preview columns that exist
        all_cols = [desc[0] for desc in rows[0].keys()] if rows else []
        # rows[0].keys() returns column names for sqlite3.Row
        all_cols = list(dict(rows[0]).keys())
        display_cols = [c for c in cols if c in all_cols]
        preview = []
        for r in rows:
            d = dict(r)
            cells = []
            for c in display_cols:
                v = d.get(c)
                if isinstance(v, str) and len(v) > 60:
                    v = v[:57] + "..."
                cells.append(v)
            preview.append(cells)
        print(table(display_cols, preview))

    conn.close()


def main():
    parser = argparse.ArgumentParser(
        description="Check capsem session DB integrity and show event summary.",
    )
    parser.add_argument(
        "session_id",
        nargs="?",
        help="Session ID to check (default: latest)",
    )
    parser.add_argument(
        "-n",
        "--rows",
        type=int,
        default=5,
        help="Number of preview rows per table (default: 5)",
    )
    parser.add_argument(
        "--list",
        action="store_true",
        help="List recent sessions from main.db and exit",
    )
    args = parser.parse_args()

    if args.list:
        sessions = list_recent_sessions(5)
        if not sessions:
            print(f"{RED}No sessions found{RESET}", file=sys.stderr)
            sys.exit(1)
        print(f"\n{BOLD}Recent sessions:{RESET}")
        headers = [
            "id", "mode", "status", "created_at", "requests",
            "in_tokens", "out_tokens", "cost", "tools", "mcp", "files",
        ]
        rows = []
        for s in sessions:
            rows.append([
                s["id"],
                s["mode"],
                s["status"],
                s["created_at"],
                f"{s['allowed_requests']}/{s['total_requests']}",
                str(s["total_input_tokens"]),
                str(s["total_output_tokens"]),
                f"${s['total_estimated_cost']:.4f}",
                str(s["total_tool_calls"]),
                str(s["total_mcp_calls"]),
                str(s["total_file_events"]),
            ])
        print(table(headers, rows))
        return

    # -- Recent sessions table --
    sessions = list_recent_sessions(5)
    if sessions:
        print(f"\n{BOLD}Recent sessions:{RESET}")
        headers = [
            "id", "mode", "status", "created_at", "requests",
            "in_tokens", "out_tokens", "cost",
        ]
        rows = []
        for s in sessions:
            rows.append([
                s["id"],
                s["mode"],
                s["status"],
                s["created_at"],
                f"{s['allowed_requests']}/{s['total_requests']}",
                str(s["total_input_tokens"]),
                str(s["total_output_tokens"]),
                f"${s['total_estimated_cost']:.4f}",
            ])
        print(table(headers, rows))

    # -- Detailed check --
    db_path = resolve_session(args.session_id)
    check_session(db_path, preview_rows=args.rows)


if __name__ == "__main__":
    main()
