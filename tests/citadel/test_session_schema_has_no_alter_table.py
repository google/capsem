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
the column it lacks -- which is how a pre-archive ledger missing
`block_offset` is already refused (`a_pre_archive_body_table_fails_to_open_by_name`
in `schema/tests.rs`).

There is no exemption. `schema/` has no `ALTER TABLE` of any form.

`schema/network_types.rs` was the last one, and for a while it was carved out
of this guard: its `ALTER TABLE ... RENAME TO` was the first half of a
copy-and-swap onto a table rebuilt from `CREATE_SCHEMA`, so unlike an `ADD
COLUMN` it could not invent a shape `ddl.rs` did not describe. That was true
and beside the point. What it rebuilt was `security_rule_events`,
`security_decision_events` and `security_ask_events` -- renamed out of the way,
copied across, dropped, with the `AUTOINCREMENT` sequence restored so the row
ids came out looking untouched. Three security ledgers rewritten on open, by
the writer, on its own initiative. The whole value of those tables is that
nobody edited them.

So the form does not matter and the carve-out is gone: `RENAME TO` is as much
a violation here as `ADD COLUMN`, and the adversarial cases below prove both.
Detection survived the burn -- `security_event_types::assert_current` runs the
same `sql.contains(...)` test the rebuild used to trigger on, and fails naming
the table instead of rewriting it.

`session_index.rs` is out of scope, and that is not an exemption from this
rule but a different file under a different one: it is the cross-session
`main.db` in `~/.capsem`, which outlives every build on a developer's machine
and is the one ledger that genuinely has to migrate. Its steps are versioned
against `user_version`, run in order, and check their results.

See CLAUDE.md 'Logger DB Boundary' and skills/dev-session-debug.
"""

from __future__ import annotations

from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
SCHEMA = PROJECT_ROOT / "crates/capsem-logger/src/schema.rs"
SCHEMA_DIR = PROJECT_ROOT / "crates/capsem-logger/src/schema"

NO_SESSION_ALTER_RATIONALE = """\
The session ledger's schema is declared once, in schema/ddl.rs.

`ALTER TABLE` in the session schema means a second place decides what a table
looks like, and every one of these was written as `let _ = conn.execute(...)`:
a locked database and an already-present column produced the same silence.
`session.db` is created per session from ddl.rs and never carried across
builds, so an older file fails at create_tables/ready() naming what it lacks.

Any form counts. `RENAME TO` is how the last one was written: it renamed three
security ledgers out of the way, rebuilt them, copied the rows back and reset
the AUTOINCREMENT sequence so the ids looked untouched.

main.db is the ledger that migrates, and its steps live in session_index.rs.

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
    """schema.rs and every production submodule. No carve-outs."""
    sources = [SCHEMA]
    if SCHEMA_DIR.is_dir():
        sources.extend(sorted(SCHEMA_DIR.rglob("*.rs")))
    return [path for path in sources if not is_test_source(path)]


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


def test_the_detection_survived_the_rebuild_it_replaced() -> None:
    """Burning a migration must not burn the check that found the problem.

    `network_types::migrate` tested `sql.contains(SECURITY_EVENT_TYPE_CHECK)`
    and then rewrote the table. The test is sound and is what tells a stale
    ledger from a current one; only the rewrite was wrong. If this assertion
    ever fails, a stale security ledger has become undetectable rather than
    unrepaired, which is strictly worse than what was burned.
    """
    checker = SCHEMA_DIR / "security_event_types.rs"
    assert checker.is_file(), (
        "the event-type check is gone; burning the rebuild was supposed to keep it"
    )
    text = checker.read_text()
    assert "SECURITY_EVENT_TYPE_CHECK" in text, "the stale-ledger test is gone"
    assert not alter_table_violations(
        str(checker.relative_to(PROJECT_ROOT)), text
    ), "the check must read, not rewrite"


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


def test_the_predicate_flags_a_reintroduced_rebuild() -> None:
    """The other adversarial case: the rename half of a copy-and-swap.

    This is the form the last migration in the session schema actually took,
    and the form a carve-out once let through.
    """
    revived = """
fn rebuild(conn: &Connection, schema: &str, table: &str, ddl: &str) -> rusqlite::Result<()> {
    let old = format!("{table}_before_network_types");
    conn.execute_batch(&format!("ALTER TABLE {schema}.{table} RENAME TO {old}"))?;
    conn.execute_batch(ddl)?;
    conn.execute_batch(&format!("ALTER TABLE {schema}.{old} RENAME TO {table}_archived"))
}
"""
    found = alter_table_violations("crates/capsem-logger/src/schema/network_types.rs", revived)
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
