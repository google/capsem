"""Check a session's body archive against the index that points into it.

Bodies live in ``session.bodies`` beside ``session.db``; the database keeps
``body_blocks`` (one row per sealed block) and ``event_body_blobs`` (one row
per body, naming its block and its span inside the block's raw bytes). A
ledger whose index and archive disagree serves nothing for the rows that
disagree, so the doctor checks the facts that must hold between them without
trusting either side:

* every body names a block the ledger recorded;
* every body's span fits inside its block's raw length;
* the file is long enough to hold every recorded block, header included,
  and starts with the archive's own header.

The byte layout is not restated here. ``tests/helpers/body_archive.py`` is the
one allowlisted Python reader of the format (see
``tests/citadel/test_body_archive_format_is_one_place.py``), so this loads it
by path and uses its constants and its hash-verifying reader.

Findings are returned rather than printed, so the report owns presentation.
"""

from __future__ import annotations

import functools
import importlib.util
import os
import sqlite3
from dataclasses import dataclass, field
from pathlib import Path
from types import ModuleType

PROJECT_ROOT = Path(os.environ.get("CAPSEM_REPOSITORY_ROOT", Path.cwd())).resolve()
BODY_ARCHIVE_HELPER = PROJECT_ROOT / "tests" / "helpers" / "body_archive.py"
ARCHIVE_TABLES = frozenset({"body_blocks", "event_body_blobs"})


@dataclass
class ArchiveFindings:
    """What the index says, what the file holds, and where they disagree."""

    bodies: int = 0
    blocks: int = 0
    archive: Path | None = None
    archive_bytes: int | None = None
    blocks_end: int = 0
    #: Bodies read back through the hash check; ``None`` when not asked.
    verified: int | None = None
    problems: list[str] = field(default_factory=list)


@functools.cache
def body_archive_helper() -> ModuleType:
    """The repository's one Python reader of the ``.bodies`` format."""
    spec = importlib.util.spec_from_file_location("capsem_body_archive", BODY_ARCHIVE_HELPER)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load the body archive reader from {BODY_ARCHIVE_HELPER}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _scalar(conn: sqlite3.Connection, sql: str) -> int:
    return int(conn.execute(sql).fetchone()[0] or 0)


def _index_problems(conn: sqlite3.Connection) -> list[str]:
    problems = []
    dangling = _scalar(
        conn,
        "SELECT COUNT(*) FROM event_body_blobs b"
        " LEFT JOIN body_blocks k ON k.block_offset = b.block_offset"
        " WHERE k.block_offset IS NULL",
    )
    if dangling:
        problems.append(f"{dangling} bodies name a block body_blocks does not record")
    overruns = _scalar(
        conn,
        "SELECT COUNT(*) FROM event_body_blobs b"
        " JOIN body_blocks k ON k.block_offset = b.block_offset"
        " WHERE b.body_offset + b.body_len > k.raw_len",
    )
    if overruns:
        problems.append(f"{overruns} bodies run past the end of their block's raw bytes")
    return problems


def _file_problems(
    findings: ArchiveFindings, archive: Path, db_path: Path, helper: ModuleType
) -> list[str]:
    if not archive.exists():
        if findings.blocks:
            return [f"{archive.name} is missing and {findings.blocks} blocks are recorded"]
        return []
    findings.archive_bytes = archive.stat().st_size
    if findings.archive_bytes < findings.blocks_end:
        return [
            f"{archive.name} is {findings.archive_bytes} bytes but its last recorded"
            f" block ends at {findings.blocks_end}"
        ]
    try:
        helper.SessionArchive(db_path).close()
    except AssertionError as error:
        return [str(error)]
    return []


def _unreadable_bodies(conn: sqlite3.Connection, db_path: Path, helper: ModuleType) -> list[str]:
    """Read every body through the hash-verifying reader, in archive order."""
    rows = conn.execute(
        "SELECT event_id, source_table, direction FROM event_body_blobs"
        " ORDER BY block_offset, body_offset"
    ).fetchall()
    failures = []
    with helper.SessionArchive(db_path) as archive:
        for event_id, source_table, direction in rows:
            try:
                archive.read(event_id, source_table, direction)
            except AssertionError as error:
                failures.append(f"{event_id} {source_table}/{direction}: {error}")
    return failures


def check_body_archive(
    conn: sqlite3.Connection, db_path: Path, *, verify_bodies: bool
) -> ArchiveFindings:
    """Compare the index with the file; optionally read every body back."""
    helper = body_archive_helper()
    archive = helper.archive_path_for_db(db_path)
    findings = ArchiveFindings(
        bodies=_scalar(conn, "SELECT COUNT(*) FROM event_body_blobs"),
        blocks=_scalar(conn, "SELECT COUNT(*) FROM body_blocks"),
        archive=archive,
        blocks_end=_scalar(
            conn,
            f"SELECT MAX(block_offset + {helper.BLOCK_HEADER_BYTES} + comp_len) FROM body_blocks",
        ),
    )
    findings.problems = _index_problems(conn) + _file_problems(findings, archive, db_path, helper)
    if verify_bodies and findings.bodies and not findings.problems:
        findings.problems = _unreadable_bodies(conn, db_path, helper)
        findings.verified = findings.bodies - len(findings.problems)
    return findings


def fs_overflow(conn: sqlite3.Connection) -> tuple[int, int]:
    """File-monitor windows that deferred changes, and how many they deferred.

    An ``overflow`` row is the monitor saying one scan saw more changes than a
    window emits and held the rest for the next scan; its ``size`` is the count.
    """
    windows, deferred = conn.execute(
        "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM fs_events WHERE action = 'overflow'"
    ).fetchone()
    return int(windows), int(deferred)
