#!/usr/bin/env python3
"""List recent Capsem sessions with per-table event counts."""

import argparse
import sqlite3
import sys
from pathlib import Path

SESSIONS_DIR = Path.home() / ".capsem" / "sessions"
MAIN_DB = SESSIONS_DIR / "main.db"

BOLD = "\033[1m"
DIM = "\033[2m"
RESET = "\033[0m"


def main():
    parser = argparse.ArgumentParser(description="List recent Capsem sessions")
    parser.add_argument("-n", type=int, default=10, help="Number of sessions (default: 10)")
    parser.add_argument("--all", action="store_true", help="Include vacuumed sessions")
    args = parser.parse_args()

    if not MAIN_DB.exists():
        print("No main.db found at", MAIN_DB, file=sys.stderr)
        sys.exit(1)

    conn = sqlite3.connect(f"file:{MAIN_DB}?mode=ro", uri=True)
    conn.row_factory = sqlite3.Row
    status_filter = "" if args.all else "WHERE status != 'vacuumed'"
    rows = conn.execute(
        f"SELECT id, status, created_at, stopped_at, storage_mode,"
        f" total_requests, allowed_requests, denied_requests,"
        f" total_input_tokens, total_output_tokens,"
        f" total_estimated_cost, total_tool_calls, total_mcp_calls,"
        f" total_file_events"
        f" FROM sessions {status_filter} ORDER BY created_at DESC LIMIT ?",
        (args.n,),
    ).fetchall()
    conn.close()

    if not rows:
        print("No sessions found")
        return

    # Header
    hdr = (
        f"{'ID':>8}  {'Status':>10}  {'Created':>19}  {'Duration':>8}  {'Cost':>7}"
        f"  {'net':>5}  {'tokens':>7}  {'tool':>5}  {'mcp':>5}  {'fs':>5}  {'Mode':>8}"
    )
    print(f"{BOLD}{hdr}{RESET}")
    print(f"{DIM}{'-' * len(hdr)}{RESET}")

    for r in rows:
        sid = r["id"]
        short_id = sid[:8]
        status = r["status"] or "?"
        created = (r["created_at"] or "")[:19]
        stopped = r["stopped_at"]
        cost = r["total_estimated_cost"] or 0
        mode = r["storage_mode"] or "?"

        # Use rollup stats from main.db (always available, even after vacuum)
        net = r["total_requests"] or 0
        tokens = (r["total_input_tokens"] or 0) + (r["total_output_tokens"] or 0)
        tool = r["total_tool_calls"] or 0
        mcp = r["total_mcp_calls"] or 0
        fs = r["total_file_events"] or 0

        # Duration
        if stopped and r["created_at"]:
            try:
                from datetime import datetime
                t0 = datetime.fromisoformat(r["created_at"].replace("Z", "+00:00"))
                t1 = datetime.fromisoformat(stopped.replace("Z", "+00:00"))
                secs = int((t1 - t0).total_seconds())
                if secs < 60:
                    dur = f"{secs}s"
                elif secs < 3600:
                    dur = f"{secs // 60}m{secs % 60}s"
                else:
                    dur = f"{secs // 3600}h{(secs % 3600) // 60}m"
            except Exception:
                dur = "?"
        else:
            dur = "running" if status == "running" else "?"

        line = (
            f"{short_id:>8}  {status:>10}  {created:>19}  {dur:>8}  ${cost:>6.2f}"
            f"  {net:>5}  {tokens:>7}  {tool:>5}  {mcp:>5}  {fs:>5}  {mode:>8}"
        )
        print(line)


if __name__ == "__main__":
    main()
