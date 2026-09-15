"""Citadel guard: a logger database's freshness is the database's to answer.

A service route cached its response bytes keyed on `session.db`'s size and
modification time. A commit that lands only in the write-ahead log changes
neither, so the route served its first snapshot until the writer's next
checkpoint -- for months, green, and only caught when a real-VM proof read
`security/latest` for rows it had just been denied. The logger's reader
already re-syncs on SQLite `data_version`; the file's metadata was a second,
wrong answer to the same question.

The rule this guard holds: outside capsem-logger, no function that names a
logger database (`session.db`, `main.db`, `network.db`, a `db_path`) may also
read filesystem metadata. Ask the `DbHandle` (its read-cache epochs) instead.
"""

from __future__ import annotations

import re

# `tests/` is on sys.path (root conftest) and is a ty search root, so the
# sibling guard resolves the same way at runtime and for the type checker.
from citadel.test_db_boundary import (
    is_logger_db_internal,
    is_test_source,
    relative,
    rust_sources,
)

FUNCTION_HEADER = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+(\w+)")
#: A freshness fingerprint needs the modification time; a size read alone is
#: how a bundle or a budget measures a file, and stays allowed.
MTIME_READS = (".modified()",)
LOGGER_DB_TOKENS = ("session.db", "main.db", "network.db", "db_path", "session_db")
#: The crates that answer ledger reads. A file packer such as the CLI's
#: support bundle handles session.db as bytes and is out of this rule's scope.
LEDGER_READER_CRATES = (
    "capsem-service",
    "capsem-gateway",
    "capsem-mcp",
    "capsem-core",
    "capsem-bench",
    "capsem-tui",
    "capsem-process",
)

RATIONALE = """\
Logger DB freshness must come from the DB object, not the file.

A function that names a logger database and reads filesystem metadata is
deciding whether the database changed from its size or mtime. WAL commits
change neither. The DbHandle owns that answer (`read_cache_epoch`, and the
reader's `data_version` sync); route code may cache only on what it says.
"""


def metadata_freshness_violations(source: str) -> list[str]:
    """Functions that read file metadata and name a logger database.

    Bodies are delimited by `fn` headers, which is coarse but sufficient: the
    mistake this guards against lives inside one helper, and a false positive
    is a function to look at, not a build to unblock.
    """
    violations: list[str] = []
    name = "<module>"
    body: list[str] = []

    def close() -> None:
        text = "\n".join(body)
        if any(read in text for read in MTIME_READS) and any(token in text for token in LOGGER_DB_TOKENS):
            violations.append(name)

    for line in source.splitlines():
        header = FUNCTION_HEADER.match(line)
        if header:
            close()
            name, body = header.group(1), []
        body.append(line)
    close()
    return violations


HISTORICAL_SHAPE = """\
pub(super) fn stats_detail_db_fingerprint(db_path: &StdPath) -> Option<String> {
    let metadata = std::fs::metadata(db_path).ok()?;
    let modified = metadata.modified().ok()?;
    Some(format!("{}:{modified:?}", metadata.len()))
}

pub(super) fn session_response_cache_get(state: &ServiceState, db_path: &StdPath) -> Option<Bytes> {
    let db_fingerprint = stats_detail_db_fingerprint(db_path)?;
    None
}
"""


def test_the_guard_catches_the_shape_that_shipped() -> None:
    assert metadata_freshness_violations(HISTORICAL_SHAPE) == ["stats_detail_db_fingerprint"]


def test_metadata_of_other_files_is_not_a_violation() -> None:
    innocent = """\
fn asset_fingerprint(path: &StdPath) -> Option<(u64, u128)> {
    let metadata = std::fs::metadata(path).ok()?;
    Some((metadata.len(), 0))
}
async fn read_rows(db_path: &StdPath) -> Vec<Row> {
    open_ready_session_db(db_path).await.query("SELECT 1").await
}
fn session_budget(dir: &StdPath) -> u64 {
    std::fs::metadata(dir.join("session.db")).map(|m| m.len()).unwrap_or(0)
}
"""
    assert metadata_freshness_violations(innocent) == []


def test_no_route_owner_judges_a_logger_database_by_its_file() -> None:
    violations = [
        f"{relative(path)}::{function}"
        for path in rust_sources()
        if relative(path).parts[1] in LEDGER_READER_CRATES
        and not is_test_source(path)
        and not is_logger_db_internal(path)
        for function in metadata_freshness_violations(path.read_text())
    ]
    assert not violations, RATIONALE + "\n" + "\n".join(violations)
