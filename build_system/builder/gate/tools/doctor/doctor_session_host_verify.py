"""Validate the host ledger, snapshots, and logs for doctor sessions."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Protocol

from capsem_builder.gate.tools.doctor.host_ledger import (
    SESSION_CREATED,
    SESSION_STOPPED,
    read_host_events,
    verify_chain,
)

BOLD = "\033[1m"
RESET = "\033[0m"


class ResultSink(Protocol):
    def ok(self, message: str) -> None: ...
    def fail(self, message: str) -> None: ...
    def check(self, condition: bool, pass_message: str, fail_message: str) -> None: ...


def _verify_host_ledger(results: ResultSink, session_id: str, host_ledger: Path) -> None:
    print(f"\n{BOLD}host ledger{RESET}")
    if not host_ledger.exists():
        results.fail(f"host ledger not found at {host_ledger}")
        return
    events = read_host_events(host_ledger)
    try:
        results.ok(f"host ledger chain intact over {verify_chain(events)} events")
    except ValueError as error:
        results.fail(str(error))
    kinds = [event["kind"] for event in events if event["session_id"] == session_id]
    results.check(
        kinds[:1] == [SESSION_CREATED] and kinds[-1:] == [SESSION_STOPPED],
        f"host ledger records {session_id}: {', '.join(kinds)}",
        f"host ledger records {session_id} as {kinds} (expected created ... stopped)",
    )


def _valid_log_entry(line: str) -> bool:
    try:
        entry = json.loads(line)
    except json.JSONDecodeError:
        return False
    message = entry.get("message")
    if message is None and "fields" in entry:
        message = entry["fields"].get("message")
    return all(key in entry for key in ("timestamp", "level", "target")) and message is not None


def _verify_log(results: ResultSink, session_dir: Path) -> None:
    print(f"\n{BOLD}log files{RESET}")
    path = session_dir / "process.log"
    results.check(
        path.exists(), f"process.log exists at {path}", f"process.log NOT found at {path}"
    )
    if not path.exists():
        return
    lines = [line for line in path.read_text().splitlines() if line.strip()]
    results.check(
        len(lines) >= 3,
        f"{len(lines)} entries in process.log",
        f"only {len(lines)} entries in process.log (expected >= 3)",
    )
    valid = sum(_valid_log_entry(line) for line in lines)
    results.check(
        valid == len(lines),
        f"all {valid} process.log entries are valid JSONL",
        f"{valid}/{len(lines)} valid JSONL entries",
    )


def verify_host_artifacts(
    results: ResultSink,
    session_id: str,
    session_dir: Path,
    host_ledger: Path,
) -> None:
    """Validate every host-side artifact after the session DB is closed."""
    _verify_host_ledger(results, session_id, host_ledger)
    _verify_log(results, session_dir)
