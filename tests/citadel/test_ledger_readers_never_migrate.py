"""Citadel guard: a reader reads. It does not change the file it is reading.

`schema/transport.rs::upgrade_legacy` ran from `DbReader::open`. On a ledger
without a `transport_schema` marker it took an IMMEDIATE transaction, created
`transport_events` and its indexes, and stamped the marker -- from a reader,
on a file another process was writing, while a route was waiting for an
answer.

That is wrong three times over. A read-only caller is not entitled to a write
lock, and taking one blocks the writer thread that owns the ledger. The
"another process may have finished the migration while this reader waited"
retry inside it is the shape of a race being managed rather than avoided. And
it made the reader a second author of the schema, beside `schema/ddl.rs`: two
places that both decide what a table looks like, which is how they disagree.

A reader now asserts instead. `assert_current` names the table or column that
is missing and fails, which is what the route's readiness contract wants:
loud, not empty, and not repaired behind the operator's back. A session ledger
belongs to the writer that created it; a file an old build wrote is that
build's, and a new one says so rather than editing it.

The same rule covers `CREATE TABLE`, whichever schema it names. Readers used
to build a RAM mirror of the hot tables in a `mem` schema; they no longer attach
`mem` at all (#213). The writer's `mem` holds only rows it has not flushed, so a
reader that read it would see a sliver of the ledger, and one that copied disk
into it would pay the whole ledger in RAM again. The reader opens the file and
nothing else, which the test below holds.

The forbidden spellings cover the idiomatic forms as well as the one that was
there. `Transaction::new` was how `upgrade_legacy` did it, but
`conn.transaction()` and `conn.unchecked_transaction()` are what someone would
reach for next, and SQL keywords are matched case-insensitively -- the burn is
about what the code does, not how it was typed.

See CLAUDE.md 'Logger DB Boundary' and skills/dev-session-debug.
"""

from __future__ import annotations

from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
LOGGER_SRC = PROJECT_ROOT / "crates/capsem-logger/src"

# Everything the read path is made of: the reader, its submodules, and the
# transport ledger's readiness check, which used to be its migration.
READ_PATH = (
    "crates/capsem-logger/src/reader.rs",
    "crates/capsem-logger/src/schema/transport.rs",
)
READ_PATH_DIR = "crates/capsem-logger/src/reader/"

FORBIDDEN: tuple[tuple[str, str], ...] = (
    ("ALTER TABLE", "a reader changing the schema it reads"),
    ("CREATE TABLE", "a reader authoring schema beside ddl.rs"),
    ("DROP TABLE", "a reader removing a table it reads"),
    # The spelling that was there, and the three anyone would reach for next.
    ("Transaction::new", "a reader taking a write transaction"),
    ("TransactionBehavior::Immediate", "a reader taking a write lock"),
    (".transaction(", "a reader opening a transaction"),
    ("unchecked_transaction(", "a reader opening a transaction"),
    ("transaction_with_behavior", "a reader choosing a lock mode"),
    ("BEGIN IMMEDIATE", "a reader taking a write lock in SQL"),
)

READERS_NEVER_MIGRATE_RATIONALE = """\
A reader reads.

`transport::upgrade_legacy` created tables from `DbReader::open`, inside an
IMMEDIATE transaction, on a file another process was writing. It is now
`assert_current`, which names the missing table or column and fails.

Do not put DDL or a write transaction back on the read path. A ledger an older
build wrote belongs to that build: say so loudly. `schema/ddl.rs` is the only
place that defines a table.
"""


def code_of(line: str) -> str:
    """The line with any `//` tail removed, so prose may explain the burn."""
    return line.split("//", 1)[0]


def matches(needle: str, line: str) -> bool:
    """SQL is matched however it was typed; Rust paths are matched exactly."""
    if needle.upper() == needle and " " in needle:
        return needle.lower() in line.lower()
    return needle in line


def read_path_violations(path: str, text: str) -> list[str]:
    """Pure predicate over (path, text): DDL or write locks on the read path."""
    if path not in READ_PATH and not path.startswith(READ_PATH_DIR):
        return []
    if path.endswith("tests.rs") or "/tests/" in path:
        return []
    return [
        f"{path}:{number} contains `{needle}` ({reason})"
        for number, line in enumerate(text.splitlines(), start=1)
        for needle, reason in FORBIDDEN
        if matches(needle, code_of(line))
    ]


def read_path_sources() -> list[Path]:
    sources = [PROJECT_ROOT / name for name in READ_PATH]
    reader_dir = PROJECT_ROOT / READ_PATH_DIR
    if reader_dir.is_dir():
        sources.extend(sorted(reader_dir.rglob("*.rs")))
    return sources


def test_the_read_path_exists() -> None:
    """A guard over files nobody has asserts nothing."""
    for name in READ_PATH:
        assert (PROJECT_ROOT / name).is_file(), f"{name} is missing; this guard is vacuous"


def test_no_reader_migrates_the_ledger_it_reads() -> None:
    violations: list[str] = []
    for source in read_path_sources():
        if not source.is_file():
            continue
        violations.extend(read_path_violations(str(source.relative_to(PROJECT_ROOT)), source.read_text()))

    assert not violations, READERS_NEVER_MIGRATE_RATIONALE + "\n" + "\n".join(violations)


def test_the_predicate_flags_the_migration_that_was_burned() -> None:
    """The adversarial case: `upgrade_legacy`, in the shape it had."""
    revived = """
pub(crate) fn upgrade_legacy(conn: &Connection) -> rusqlite::Result<()> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS transport_events (id INTEGER);")?;
    tx.execute_batch("ALTER TABLE transport_events ADD COLUMN network_id TEXT;")?;
    tx.commit()
}
"""
    found = read_path_violations("crates/capsem-logger/src/schema/transport.rs", revived)
    assert len(found) == 4, found

    # The same code somewhere off the read path is this guard's business only
    # through its own rule; here it is simply out of scope.
    assert read_path_violations("crates/capsem-logger/src/writer.rs", revived) == []


def test_the_predicate_flags_the_idiomatic_spellings() -> None:
    """The burn is about what the code does, not how someone typed it."""
    for line, expected in (
        ("    let tx = conn.transaction()?;", ".transaction("),
        ("    let tx = conn.unchecked_transaction()?;", "unchecked_transaction("),
        ("    let tx = conn.transaction_with_behavior(Immediate)?;", "transaction_with_behavior"),
        ('    conn.execute_batch("begin immediate")?;', "BEGIN IMMEDIATE"),
        ('    conn.execute_batch("alter table t add column c TEXT")?;', "ALTER TABLE"),
        ('    conn.execute_batch("create table if not exists t (id INTEGER)")?;', "CREATE TABLE"),
    ):
        found = read_path_violations("crates/capsem-logger/src/reader.rs", line)
        assert any(expected in item for item in found), (line, found)


NO_READER_MIRROR_RATIONALE = """\
A reader reads the file, not the writer's memory.

The writer's `mem` schema holds only rows it has not flushed yet. Readers used
to attach it and read hot tables through TEMP views, which obliged the writer
to rehydrate the whole ledger into RAM at open and keep every row for the life
of the session: capsem-process grew ~138 KB per request (#213).

Do not attach `mem`, create memory tables or views, or copy disk into memory
from the read path. A reader that needs freshness asks SQLite's `data_version`.
"""

READER_MIRROR_CALLS = (
    "create_memory_tables",
    "create_memory_read_views",
    "rehydrate_memory_tables",
    "sync_memory_tables_from_disk",
    "memory_uri_for",
    "MEMORY_SCHEMA",
)


def reader_mirror_violations(path: str, text: str) -> list[str]:
    """Pure predicate over (path, text): the read path reaching for `mem`."""
    if path not in READ_PATH and not path.startswith(READ_PATH_DIR):
        return []
    if path.endswith("tests.rs") or "/tests/" in path:
        return []
    return [
        f"{path}:{number} contains `{needle}`"
        for number, line in enumerate(text.splitlines(), start=1)
        for needle in READER_MIRROR_CALLS
        if needle in code_of(line)
    ]


def test_no_reader_attaches_the_writers_memory() -> None:
    opener = PROJECT_ROOT / "crates/capsem-logger/src/reader/open.rs"
    assert opener.is_file(), f"{opener} is missing; this guard is vacuous"
    violations: list[str] = []
    for source in read_path_sources():
        if source.is_file():
            violations.extend(reader_mirror_violations(str(source.relative_to(PROJECT_ROOT)), source.read_text()))
    assert not violations, NO_READER_MIRROR_RATIONALE + "\n" + "\n".join(violations)


def test_the_mirror_predicate_flags_the_reader_that_was_burned() -> None:
    """The adversarial case: `DbReader::open_with` as it was before #213."""
    revived = """
    if memory_mirror {
        let memory_uri = schema::memory_uri_for_path(path);
        schema::with_memory_schema_lock(|| {
            schema::create_memory_tables(&conn, &memory_uri)?;
            schema::rehydrate_memory_tables_from_disk_once(&conn, schema::hot_ledger_tables())?;
            schema::create_memory_read_views(&conn)
        })?;
    }
"""
    found = reader_mirror_violations("crates/capsem-logger/src/reader/open.rs", revived)
    assert len(found) == 4, found
    # The writer owns `mem`; this guard does not reach it.
    assert reader_mirror_violations("crates/capsem-logger/src/writer.rs", revived) == []


def test_the_predicate_allows_an_assertion() -> None:
    """The legitimate shape: read the shape, name what is missing, fail."""
    honest = """
pub(crate) fn assert_current(conn: &Connection) -> rusqlite::Result<()> {
    let columns = table_column_names(conn, "main", "transport_events")?;
    for column in required {
        if !columns.iter().any(|actual| actual == column) {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "transport_events missing required column {column}"
            )));
        }
    }
    Ok(())
}
"""
    assert read_path_violations("crates/capsem-logger/src/schema/transport.rs", honest) == []
