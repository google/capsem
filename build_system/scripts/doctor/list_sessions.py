#!/usr/bin/env python3
"""List recent Capsem sessions with per-table event counts."""

import argparse
import os
import sqlite3
import sys
from datetime import datetime
from pathlib import Path

CAPSEM_HOME = Path(os.environ.get("CAPSEM_HOME", Path.home() / ".capsem"))
RUN_DIR = Path(os.environ.get("CAPSEM_RUN_DIR", CAPSEM_HOME / "run"))
MAIN_DB = CAPSEM_HOME / "sessions" / "main.db"
# Ephemeral ledgers live under run/sessions/<id>; a named VM keeps its ledger
# under run/persistent/<name> across stops. main.db is the only thing left in
# ~/.capsem/sessions, so looking for ledgers there found none.
LEDGER_DIRS = (RUN_DIR / "sessions", RUN_DIR / "persistent")


def has_ledger(session_id: str) -> bool:
    return any((root / session_id / "session.db").exists() for root in LEDGER_DIRS)


BOLD = "\033[1m"
DIM = "\033[2m"
RESET = "\033[0m"


def fmt_duration(created_at, stopped_at, status):
    if not stopped_at or not created_at:
        return "running" if status == "running" else "?"
    try:
        t0 = datetime.fromisoformat(created_at.replace("Z", "+00:00"))
        t1 = datetime.fromisoformat(stopped_at.replace("Z", "+00:00"))
        secs = int((t1 - t0).total_seconds())
        if secs < 60:
            return f"{secs}s"
        if secs < 3600:
            return f"{secs // 60}m{secs % 60}s"
        return f"{secs // 3600}h{(secs % 3600) // 60}m"
    except Exception:
        return "?"


def main():
    parser = argparse.ArgumentParser(description="List recent Capsem sessions")
    parser.add_argument("-n", type=int, default=10, help="Number of sessions (default: 10)")
    parser.add_argument("--with-db", action="store_true", help="Only sessions with session.db on disk")
    parser.add_argument("--with-model", action="store_true", help="Only sessions with model calls (tokens > 0)")
    parser.add_argument("--with-net", action="store_true", help="Only sessions with network events")
    parser.add_argument("--with-tools", action="store_true", help="Only sessions with tool calls")
    parser.add_argument("--min-cost", type=float, default=0, help="Minimum estimated cost")
    args = parser.parse_args()

    if not MAIN_DB.exists():
        print("No main.db found at", MAIN_DB, file=sys.stderr)
        sys.exit(1)

    conn = sqlite3.connect(f"file:{MAIN_DB}?mode=ro", uri=True)
    conn.row_factory = sqlite3.Row

    conditions = []
    if args.with_model:
        conditions.append("(total_input_tokens + total_output_tokens) > 0")
    if args.with_net:
        conditions.append("total_requests > 0")
    if args.with_tools:
        conditions.append("total_tool_calls > 0")
    if args.min_cost > 0:
        conditions.append(f"total_estimated_cost >= {args.min_cost}")

    where = f"WHERE {' AND '.join(conditions)}" if conditions else ""

    rows = conn.execute(
        f"SELECT id, status, created_at, stopped_at, storage_mode,"
        f" total_requests, allowed_requests, denied_requests,"
        f" total_input_tokens, total_output_tokens,"
        f" total_estimated_cost, total_tool_calls, total_file_events"
        f" FROM sessions {where} ORDER BY created_at DESC LIMIT ?",
        (args.n,),
    ).fetchall()
    conn.close()

    # Post-filter for --with-db (needs filesystem check)
    if args.with_db:
        rows = [r for r in rows if has_ledger(r["id"])]

    if not rows:
        print("No sessions found")
        return

    # Header
    hdr = (
        f"{'ID':<36}  {'Created':>16}  {'Dur':>6}  {'Cost':>7}"
        f"  {'net':>5}  {'tokens':>7}  {'tool':>5}  {'fs':>5}"
    )
    print(f"{BOLD}{hdr}{RESET}")
    print(f"{DIM}{'-' * len(hdr)}{RESET}")

    for r in rows:
        sid = r["id"]
        status = r["status"] or "?"
        created = (r["created_at"] or "")
        # Show date as MM-DD HH:MM:SS (compact)
        try:
            dt = datetime.fromisoformat(created.replace("Z", "+00:00"))
            created_short = dt.strftime("%m-%d %H:%M:%S")
        except Exception:
            created_short = created[:16]

        dur = fmt_duration(r["created_at"], r["stopped_at"], status)
        cost = r["total_estimated_cost"] or 0
        net = r["total_requests"] or 0
        tokens = (r["total_input_tokens"] or 0) + (r["total_output_tokens"] or 0)
        tool = r["total_tool_calls"] or 0
        fs = r["total_file_events"] or 0

        # Mark sessions that still have DB on disk
        has_db = "*" if has_ledger(sid) else " "

        line = (
            f"{sid:<36}{has_db} {created_short:>16}  {dur:>6}  ${cost:>6.2f}"
            f"  {net:>5}  {tokens:>7}  {tool:>5}  {fs:>5}"
        )
        print(line)

    print(f"\n{DIM}* = session.db on disk (queryable){RESET}")


if __name__ == "__main__":
    main()
