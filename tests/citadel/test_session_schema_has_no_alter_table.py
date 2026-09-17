"""Citadel guard: the session ledger's schema is declared once, in ddl.rs.

`schema::migrate` was seventy-odd statements of the form

    let _ = conn.execute("ALTER TABLE tool_calls ADD COLUMN origin TEXT ...", []);

one per column the schema had ever grown, each with its result discarded. On a
current file every one of them failed and was ignored; on an older file some
succeeded and produced a table whose columns matched no declaration anywhere --
a third shape, beside `ddl.rs`'s and the old build's. The discarded result is
the whole problem: a real failure (a locked database, a corrupt file) was
indistinguishable from "the column is already there".

The session ledger does not need it. `session.db` is created per session by
the writer that owns it, from `schema/ddl.rs`, and is never carried across
builds. A file an older build wrote fails at `create_tables`/`ready()` naming
the column it lacks -- which is how Task 5 already detects a pre-archive ledger
missing `block_offset`.

Two files are exempt, and the exemptions are the point.

`session_index.rs` is the cross-session `main.db`. It lives in `~/.capsem` and
outlives every build on a developer's machine, so it is the one ledger that
genuinely migrates. Its steps are versioned against `user_version`, run in
order, and check their results.

`schema/network_types.rs` rebuilds three security tables whose `event_type`
CHECK predates the current list. Its `ALTER TABLE ... RENAME TO` is the first
half of a copy-and-swap, and the table that replaces the old one comes from
`CREATE_SCHEMA` -- so unlike an `ADD COLUMN`, it cannot produce a shape that
`ddl.rs` does not describe. That is why the exemption is written as "no ADD
COLUMN here either", asserted below rather than taken on trust. It is still a
migration living in the session schema, and it is the one left to decide
about.

See CLAUDE.md 'Logger DB Boundary' and skills/dev-session-debug.
"""

from __future__ import annotations

from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
SCHEMA = PROJECT_ROOT / "crates/capsem-logger/src/schema.rs"
SCHEMA_DIR = PROJECT_ROOT / "crates/capsem-logger/src/schema"
NETWORK_TYPES = SCHEMA_DIR / "network_types.rs"

NO_SESSION_ALTER_RATIONALE = """\
The session ledger's schema is declared once, in schema/ddl.rs.

`ALTER TABLE` in the session schema means a second place decides what a table
looks like, and every one of these was written as `let _ = conn.execute(...)`:
a locked database and an already-present column produced the same silence.
`session.db` is created per session from ddl.rs and never carried across
builds, so an older file fails at create_tables/ready() naming what it lacks.

main.db is the ledger that migrates, and its steps live in session_index.rs.
schema/network_types.rs rebuilds a table by copy-and-swap, never by ADD COLUMN.

Test modules are out of scope: they build an older shape on purpose, so that
the loud failure has something to be loud about.
"""


def code_of(line: str) -> str:
    """The line with any `//` tail removed, so prose may explain the burn."""
    return line.split("//", 1)[0]


def alter_table_violations(path: str, text: str) -> list[str]:
    """Pure predicate over (path, text): session-schema ALTERs."""
    return [
        f"{path}:{number} contains `ALTER TABLE`"
        for number, line in enumerate(text.splitlines(), start=1)
        if "ALTER TABLE" in code_of(line)
    ]


def is_test_source(path: Path) -> bool:
    """A test may build an older shape on purpose.

    The tests that survived this burn do exactly that: they take a current
    database and `ALTER TABLE ... DROP COLUMN` one column back out of it, so
    that the loud failure has something to be loud about. Forbidding that
    would leave the burn with no executable proof, which is the one thing
    worse than the migration it replaced. What the guard is for is the
    production path, where a second definition of a table can hide.
    """
    return path.name == "tests.rs" or "tests" in path.parts


def session_schema_sources() -> list[Path]:
    """schema.rs and its production submodules, less the one exemption."""
    sources = [SCHEMA]
    if SCHEMA_DIR.is_dir():
        sources.extend(sorted(SCHEMA_DIR.rglob("*.rs")))
    return [
        path
        for path in sources
        if not is_test_source(path) and path != NETWORK_TYPES
    ]


def test_the_session_schema_exists() -> None:
    """A guard over a file nobody has asserts nothing."""
    assert SCHEMA.is_file(), f"{SCHEMA} is missing; this guard is vacuous"
    ddl = SCHEMA_DIR / "ddl.rs"
    assert "CREATE TABLE" in ddl.read_text(), f"{ddl} no longer declares the schema; this guard is vacuous"


def test_the_session_schema_has_no_alter_table() -> None:
    violations: list[str] = []
    for source in session_schema_sources():
        violations.extend(alter_table_violations(str(source.relative_to(PROJECT_ROOT)), source.read_text()))

    assert not violations, NO_SESSION_ALTER_RATIONALE + "\n" + "\n".join(violations)


def test_main_db_is_the_ledger_that_migrates() -> None:
    """The exemption, asserted rather than assumed.

    If `session_index.rs` ever stops migrating, this guard's rationale is
    wrong and the exemption should go with it.
    """
    index = PROJECT_ROOT / "crates/capsem-logger/src/session_index.rs"
    text = index.read_text()
    assert "ALTER TABLE" in text, "main.db no longer migrates; revisit this guard's exemption"
    assert "user_version" in text, "main.db's migrations must be versioned"


def test_the_network_types_rebuild_never_adds_a_column() -> None:
    """The exemption, asserted rather than assumed.

    `network_types.rs` is allowed `ALTER TABLE` because its only use is the
    rename half of a copy-and-swap onto a table built from `CREATE_SCHEMA`.
    The moment it grows an `ADD COLUMN`, it is deciding what a column is --
    the exact thing burned out of `migrate` -- and the exemption stops
    applying to it.
    """
    text = NETWORK_TYPES.read_text()
    assert "RENAME TO" in text, "network_types.rs no longer rebuilds; drop this exemption"
    assert "ADD COLUMN" not in text, (
        NO_SESSION_ALTER_RATIONALE
        + "\nnetwork_types.rs added a column; it is exempt from ALTER TABLE only as a rebuild"
    )


def test_the_predicate_flags_the_migration_that_was_burned() -> None:
    """The adversarial case: the discarded-result ALTER, as it was written."""
    revived = """
pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let _ = conn.execute("ALTER TABLE tool_calls ADD COLUMN origin TEXT NOT NULL DEFAULT 'native'", []);
    let _ = conn.execute(&format!("ALTER TABLE {tbl} ADD COLUMN turn_id TEXT"), []);
    Ok(())
}
"""
    found = alter_table_violations("crates/capsem-logger/src/schema.rs", revived)
    assert len(found) == 2, found


def test_a_test_module_may_build_an_older_shape() -> None:
    """The exclusion, asserted rather than assumed."""
    assert is_test_source(SCHEMA_DIR / "tests.rs")
    assert is_test_source(SCHEMA_DIR / "tests" / "transport.rs")
    assert not is_test_source(SCHEMA)
    assert not is_test_source(SCHEMA_DIR / "ddl.rs")


def test_the_predicate_allows_a_declaration() -> None:
    """The legitimate shape: the table, declared once, with its indexes."""
    honest = """
CREATE TABLE IF NOT EXISTS tool_calls (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    origin TEXT NOT NULL DEFAULT 'native'
);
CREATE INDEX IF NOT EXISTS idx_tool_calls_turn_id ON tool_calls(turn_id);
"""
    assert alter_table_violations("crates/capsem-logger/src/schema/ddl.rs", honest) == []
