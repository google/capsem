"""Check session DB integrity and show a summary of recorded events."""

import argparse
import gzip
import os
import sqlite3
import sys
import tempfile
from pathlib import Path

from capsem_builder.gate.tools.doctor.check_session_report import (
    BOLD,
    DIM,
    RED,
    RESET,
    check_session,
    table,
)

CAPSEM_HOME = Path(os.environ.get("CAPSEM_HOME", Path.home() / ".capsem"))
RUN_DIR = Path(os.environ.get("CAPSEM_RUN_DIR", CAPSEM_HOME / "run"))
SESSIONS_DIR = RUN_DIR / "sessions"
MAIN_DB = CAPSEM_HOME / "sessions" / "main.db"


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
        " total_estimated_cost, total_tool_calls, total_file_events"
        " FROM sessions ORDER BY created_at DESC LIMIT ?",
        (n,),
    ).fetchall()
    conn.close()
    return [dict(r) for r in rows]


def resolve_session(session_id: str | None) -> Path:
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
        tmp = tempfile.NamedTemporaryFile(suffix=".db", delete=False)  # noqa: SIM115 -- handed to Popen; must outlive this statement
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
            "id",
            "mode",
            "status",
            "created_at",
            "requests",
            "in_tokens",
            "out_tokens",
            "cost",
            "tools",
            "files",
        ]
        rows = []
        for s in sessions:
            rows.append(
                [
                    s["id"],
                    s["mode"],
                    s["status"],
                    s["created_at"],
                    f"{s['allowed_requests']}/{s['total_requests']}",
                    str(s["total_input_tokens"]),
                    str(s["total_output_tokens"]),
                    f"${s['total_estimated_cost']:.4f}",
                    str(s["total_tool_calls"]),
                    str(s["total_file_events"]),
                ]
            )
        print(table(headers, rows))
        return

    # -- Recent sessions table --
    sessions = list_recent_sessions(5)
    if sessions:
        print(f"\n{BOLD}Recent sessions:{RESET}")
        headers = [
            "id",
            "mode",
            "status",
            "created_at",
            "requests",
            "in_tokens",
            "out_tokens",
            "cost",
        ]
        rows = []
        for s in sessions:
            rows.append(
                [
                    s["id"],
                    s["mode"],
                    s["status"],
                    s["created_at"],
                    f"{s['allowed_requests']}/{s['total_requests']}",
                    str(s["total_input_tokens"]),
                    str(s["total_output_tokens"]),
                    f"${s['total_estimated_cost']:.4f}",
                ]
            )
        print(table(headers, rows))

    # -- Detailed check --
    db_path = resolve_session(args.session_id)
    check_session(db_path, preview_rows=args.rows)


if __name__ == "__main__":
    main()
