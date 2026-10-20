"""Check session DB integrity and show a summary of recorded events."""

import argparse
import os
import sys
from pathlib import Path

from capsem_builder.gate.tools.doctor.check_session_report import (
    BOLD,
    RED,
    RESET,
    check_session,
    table,
)
from capsem_builder.gate.tools.doctor.host_ledger import host_ledger_path, recent_sessions

CAPSEM_HOME = Path(os.environ.get("CAPSEM_HOME", Path.home() / ".capsem"))
RUN_DIR = Path(os.environ.get("CAPSEM_RUN_DIR", CAPSEM_HOME / "run"))
SESSIONS_DIR = RUN_DIR / "sessions"
PERSISTENT_DIR = RUN_DIR / "persistent"
HOST_LEDGER = host_ledger_path(CAPSEM_HOME)


def list_recent_sessions(n: int = 5) -> list[dict]:
    """Return the N most recent sessions from the host ledger."""
    if not HOST_LEDGER.exists():
        print(f"{RED}host ledger not found at {HOST_LEDGER}{RESET}", file=sys.stderr)
        sys.exit(1)
    return recent_sessions(HOST_LEDGER, n)


def resolve_session(session_id: str | None) -> Path:
    """Resolve a session ID (or latest) to its session.db path.

    Ephemeral sessions live under ``run/sessions/``; a named VM keeps its
    ledger under ``run/persistent/<name>/`` across stops.
    """
    if not session_id:
        sessions = list_recent_sessions(1)
        if not sessions:
            print(f"{RED}No sessions found in the host ledger{RESET}", file=sys.stderr)
            sys.exit(1)
        session_id = sessions[0]["id"]

    for session_dir in (SESSIONS_DIR / session_id, PERSISTENT_DIR / session_id):
        db = session_dir / "session.db"
        if db.exists():
            return db

    print(f"{RED}session.db not found for {session_id}{RESET}", file=sys.stderr)
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
        help="List recent sessions from the host ledger and exit",
    )
    parser.add_argument(
        "--db",
        type=Path,
        help="Check this session.db directly instead of resolving one from the host ledger",
    )
    parser.add_argument(
        "--verify-bodies",
        action="store_true",
        help="Read every archived body back and check it against its recorded hash",
    )
    args = parser.parse_args()

    if args.list:
        sessions = list_recent_sessions(5)
        if not sessions:
            print(f"{RED}No sessions found{RESET}", file=sys.stderr)
            sys.exit(1)
        print(f"\n{BOLD}Recent sessions:{RESET}")
        headers = ["id", "status", "created_at_ms", "stopped_at_ms"]
        rows = [
            [s["id"], s["status"], str(s["created_at_ms"]), str(s["stopped_at_ms"] or "-")]
            for s in sessions
        ]
        print(table(headers, rows))
        return 0

    if args.db is not None:
        return 0 if check_session(args.db, args.rows, verify_bodies=args.verify_bodies) else 1

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
    return 0 if check_session(db_path, args.rows, verify_bodies=args.verify_bodies) else 1


if __name__ == "__main__":
    main()
