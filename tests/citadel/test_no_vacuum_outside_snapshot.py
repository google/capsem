"""Citadel guard: the only VACUUM left is the snapshot's VACUUM INTO.

Capsem carried a whole session lifecycle nothing ever ran. A stopped session
was supposed to become `vacuumed`: `vacuum_and_compress_session_db` would
checkpoint the ledger, VACUUM it, gzip it to `session.db.gz`, delete the
originals, and `mark_vacuumed` would record a `vacuumed_at` and a
`compressed_size_bytes` in `main.db`. Nothing called it. The status existed in
`main.db`'s schema, in every retention query's `status IN (...)` list, in a
gateway type, in the doctor's session listing and in the integration tests'
accepted-status set -- a state no session could reach, which every reader of
that code still had to account for.

It cannot come back as written. Bodies now live in `session.bodies`, an
append-only archive the ledger's `event_body_blobs` rows index by block
offset. VACUUM rewrites the database file; gzipping `session.db` and deleting
it leaves that index unreadable and the archive beside it orphaned. Retention
for the archive era is a different mechanism, and it will be written against
the archive rather than by reviving a dead status.

What stays is `snapshot_session_ledger`'s `VACUUM INTO`, which is not
compaction at all: it is SQLite's supported way to clone a consistent database
to a new file, and the snapshot copies the archive after it for the same
reason.

See CLAUDE.md 'Logger DB Boundary' and skills/dev-session-debug.
"""

from __future__ import annotations

import subprocess
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
CRATES_DIR = PROJECT_ROOT / "crates"

# Where the statement keyword may still appear, and why. Anywhere else is a
# compaction path coming back.
VACUUM_ALLOWED: dict[str, str] = {
    # The snapshot clone. `VACUUM INTO` writes a second file and leaves the
    # source untouched; only this spelling is permitted here.
    "crates/capsem-logger/src/db/maintenance.rs": "snapshot_session_ledger's VACUUM INTO",
    # The raw-query validator refuses the statement: naming it is how it is
    # refused.
    "crates/capsem-logger/src/reader.rs": "validate_select_only's rejected-statement list",
    "crates/capsem-logger/tests/roundtrip/reader_queries.rs": "the test of that refusal",
}

# The one file where the spelling is also constrained.
SNAPSHOT_SOURCE = "crates/capsem-logger/src/db/maintenance.rs"

# Trees that describe the product today. `web/docs/src/content/docs/releases`
# is release history and says what shipped; it is not rewritten.
VACUUMED_ROOTS = ("crates", "web", "build_system")
VACUUMED_EXEMPT_PREFIX = "web/docs/src/content/docs/releases/"

# `main.db` survives every build on a developer's machine, so something has to
# name the dead state in order to remove it. These two files may -- but only to
# migrate, never to query: a `status IN (...)` list here is the retention shape
# this change deleted coming back.
VACUUMED_MIGRATION: dict[str, str] = {
    "crates/capsem-logger/src/session_index.rs": "the v7->v8 migration drops the columns and rewrites the state",
    "crates/capsem-logger/src/session_index/tests.rs": "the test of that migration",
}

VACUUM_RATIONALE = """\
The session ledger has no compaction path.

`vacuum_and_compress_session_db` and `checkpoint_and_vacuum_session_db` were
deleted: nothing called them, and VACUUM plus gzip would now orphan
`session.bodies` and leave `event_body_blobs`' block offsets pointing into a
file that is gone. Retention belongs against the archive.

`snapshot_session_ledger`'s `VACUUM INTO` is the exception: it clones to a new
file rather than rewriting one.
"""

VACUUMED_RATIONALE = """\
`vacuumed` is not a session state.

The status, `vacuumed_at` and `compressed_size_bytes` were removed from
main.db, the gateway types, the doctor listing and the test helpers. Do not
reintroduce a state no session can reach: every retention query had to carry
it in a `status IN (...)` list for a lifecycle nothing ran.
"""


def code_of(line: str) -> str:
    """The line with any `//` or `#` comment tail removed.

    The guard is about what the code does. The prose in this repository has to
    be able to explain the compaction path it burned, and this file's own
    module docs name it.
    """
    for marker in ("//", "#"):
        line = line.split(marker, 1)[0]
    return line


def tracked_files() -> list[Path]:
    """Every Git-tracked file, so build outputs and caches are out of scope."""
    listed = subprocess.run(
        ["git", "-C", str(PROJECT_ROOT), "ls-files", "-z"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return [PROJECT_ROOT / name for name in listed.split("\0") if name]


def vacuum_violations(path: str, text: str) -> list[str]:
    """Pure predicate over (path, text): every VACUUM that is not the snapshot."""
    allowed = VACUUM_ALLOWED.get(path)
    violations: list[str] = []
    for number, line in enumerate(text.splitlines(), start=1):
        code = code_of(line)
        if "VACUUM" not in code:
            continue
        if allowed is None:
            violations.append(f"{path}:{number} runs VACUUM outside the snapshot")
        elif path == SNAPSHOT_SOURCE and "VACUUM INTO" not in code:
            violations.append(f"{path}:{number} is a bare VACUUM in the snapshot source")
    return violations


def vacuumed_state_violations(path: str, text: str) -> list[str]:
    """Pure predicate over (path, text): every mention of the dead session state."""
    if not path.startswith(VACUUMED_ROOTS) or path.startswith(VACUUMED_EXEMPT_PREFIX):
        return []
    migrating = path in VACUUMED_MIGRATION
    return [
        f"{path}:{number} names the `vacuumed` session state"
        for number, line in enumerate(text.splitlines(), start=1)
        if "vacuumed" in line and not (migrating and "status IN" not in line)
    ]


def test_the_snapshot_source_exists() -> None:
    """A guard over a file nobody has asserts nothing."""
    snapshot = PROJECT_ROOT / SNAPSHOT_SOURCE
    assert "VACUUM INTO" in snapshot.read_text(), f"{snapshot} no longer snapshots; this guard is vacuous"


def test_no_vacuum_outside_the_snapshot() -> None:
    violations: list[str] = []
    for path in sorted(CRATES_DIR.rglob("*.rs")):
        relative = str(path.relative_to(PROJECT_ROOT))
        violations.extend(vacuum_violations(relative, path.read_text()))

    assert not violations, VACUUM_RATIONALE + "\n" + "\n".join(violations)


def test_vacuumed_is_not_a_session_state_anywhere() -> None:
    violations: list[str] = []
    for path in tracked_files():
        if not path.is_file():
            continue
        try:
            text = path.read_text()
        except (UnicodeDecodeError, OSError):
            continue
        violations.extend(vacuumed_state_violations(str(path.relative_to(PROJECT_ROOT)), text))

    assert not violations, VACUUMED_RATIONALE + "\n" + "\n".join(violations)


def test_the_predicate_flags_the_compaction_that_was_burned() -> None:
    """The adversarial case: the lifecycle, in the shapes it had."""
    revived = """
pub fn checkpoint_and_vacuum_session_db(path: &Path) -> anyhow::Result<()> {
    let conn = rusqlite::Connection::open(path)?;
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
    conn.execute_batch("VACUUM")?;
    Ok(())
}
"""
    assert vacuum_violations("crates/capsem-core/src/session/maintenance.rs", revived) == [
        "crates/capsem-core/src/session/maintenance.rs:5 runs VACUUM outside the snapshot"
    ]

    # Even in the file that is allowed to say it, only the clone spelling is.
    bare_in_the_snapshot = 'conn.execute_batch("VACUUM")?;'
    assert len(vacuum_violations(SNAPSHOT_SOURCE, bare_in_the_snapshot)) == 1

    # Even in the file allowed to migrate the state, a query naming it is the
    # retention shape coming back.
    state = "WHERE status IN ('stopped', 'crashed', 'vacuumed') AND persistent = 0"
    assert len(vacuumed_state_violations("crates/capsem-logger/src/session_index.rs", state)) == 1
    assert len(vacuumed_state_violations("crates/capsem-service/src/ledger_routes.rs", state)) == 1
    assert len(vacuumed_state_violations("web/app/src/lib/types/gateway.ts", "vacuumed_at: string;")) == 1


def test_the_predicate_allows_the_snapshot_and_the_refusal() -> None:
    """The legitimate shapes: a clone, and a validator naming what it rejects."""
    clone = """src_conn.execute_batch(&format!("VACUUM INTO '{escaped}';"))?;"""
    assert vacuum_violations(SNAPSHOT_SOURCE, clone) == []

    refusal = '"VACUUM" | "REINDEX" | "BEGIN" => return Err("statement not allowed".into()),'
    assert vacuum_violations("crates/capsem-logger/src/reader.rs", refusal) == []

    history = "- Numerous snapshot, vacuum, and telemetry fixes"
    assert vacuumed_state_violations("web/docs/src/content/docs/releases/0-14.md", history) == []

    migration = "conn.execute(\"UPDATE sessions SET status = 'stopped' WHERE status = 'vacuumed'\", [])?;"
    assert vacuumed_state_violations("crates/capsem-logger/src/session_index.rs", migration) == []
