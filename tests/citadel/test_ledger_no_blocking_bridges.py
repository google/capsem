"""Citadel guard: the ledger's read side has no blocking bridge.

`DbHandle` carried three methods that existed only to let synchronous route
code reach a session ledger while the routes were being moved behind async
handles: `ready_blocking`, `query_raw_blocking` and `with_reader_blocking`.
Each one opened a fresh `DbReader` on the calling thread, which on a route is
an async runtime worker: a file open, a schema check and a query run to
completion where nothing may block.

They were worse than slow. A bridge opens its own connection, so it sees
neither the handle's read-cache epoch nor the reader worker's `data_version`
sync -- the two things that make a route's answer current. A route served
through one could return rows the ledger had already moved past, which is the
exact failure `test_db_freshness_boundary.py` was written for.

The routes moved. Nothing called them. Deleting a transitional path once the
transition is over is the whole point of calling it transitional, and this
guard is what keeps the next one from being added back "just for this route".

`DbWriter::write_blocking` and `shutdown_blocking` are deliberately not
covered: they are the producer side, they enqueue into a bounded channel the
writer thread owns rather than opening a connection, and synchronous producers
(the guest bridge, the fixture replay) have no runtime to await on. This guard
is about the read side reaching around the handle that owns it.

See CLAUDE.md 'Logger DB Boundary' and skills/dev-rust-patterns.
"""

from __future__ import annotations

import re
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
LOGGER_SRC = PROJECT_ROOT / "crates/capsem-logger/src"

# The read-side types. A blocking bridge on any of them reaches around the
# handle that owns cache invalidation and freshness.
GUARDED_TYPES = ("DbHandle", "DbReader")

NO_BLOCKING_BRIDGE_RATIONALE = """\
The session ledger's read side has no blocking bridge.

`DbHandle::ready_blocking`, `query_raw_blocking` and `with_reader_blocking`
were deleted once the routes behind them became async. Do not add another:
each opened its own `DbReader`, so it blocked a runtime worker on file I/O and
answered from a connection that sees neither the handle's read-cache epoch nor
the reader worker's data_version sync -- a route that can serve rows the
ledger has already moved past.

Use `ready().await`, `query(...).await`, or add the read to the DB object.
`DbWriter`'s blocking producer methods are not this: they enqueue, they do not
open a connection.
"""

IMPL_RE = re.compile(r"^impl(?:<[^>]*>)?\s+(\w+)\b[^{]*\{", re.MULTILINE)
PUB_FN_RE = re.compile(r"^\s*pub(?:\([^)]*\))?\s+(?:async\s+)?fn\s+(\w+)", re.MULTILINE)


def impl_blocks(text: str) -> list[tuple[str, str]]:
    """Every `impl Type { ... }` in the source, as (type name, body).

    Brace-counted so a nested block inside a method cannot end the impl early.
    """
    blocks: list[tuple[str, str]] = []
    for match in IMPL_RE.finditer(text):
        depth = 0
        start = match.end() - 1
        end = len(text)
        for index in range(start, len(text)):
            if text[index] == "{":
                depth += 1
            elif text[index] == "}":
                depth -= 1
                if depth == 0:
                    end = index
                    break
        blocks.append((match.group(1), text[start + 1 : end]))
    return blocks


def blocking_bridges(path: str, text: str) -> list[str]:
    """Pure predicate over (path, text): every blocking bridge on a read type."""
    found: list[str] = []
    for type_name, body in impl_blocks(text):
        if type_name not in GUARDED_TYPES:
            continue
        for method in PUB_FN_RE.findall(body):
            if method.endswith("_blocking"):
                found.append(f"{path}: {type_name}::{method}")
    return found


def test_the_logger_source_exists() -> None:
    """A guard over a tree nobody has asserts nothing."""
    assert (LOGGER_SRC / "db.rs").is_file(), f"{LOGGER_SRC}/db.rs is missing; this guard is vacuous"


def test_the_read_side_has_no_blocking_bridge() -> None:
    found: list[str] = []
    for path in sorted(LOGGER_SRC.rglob("*.rs")):
        found.extend(blocking_bridges(str(path.relative_to(PROJECT_ROOT)), path.read_text()))
    assert not found, NO_BLOCKING_BRIDGE_RATIONALE + "\n" + "\n".join(found)


def test_the_predicate_flags_the_bridges_that_were_burned() -> None:
    """The adversarial case: the three methods, in their original shape."""
    adversarial = """
impl DbHandle {
    pub fn ready_blocking(&self) -> rusqlite::Result<()> { Ok(()) }
    pub fn query_raw_blocking(&self, sql: &str) -> Result<String, String> {
        if sql.is_empty() { return Err("empty".into()); }
        Ok(String::new())
    }
    pub fn with_reader_blocking<T>(&self, f: impl FnOnce(&DbReader) -> T) -> T { f(self) }
    pub async fn ready(&self) -> DbResult<()> { Ok(()) }
}

impl DbReader {
    pub fn open_blocking(path: &Path) -> rusqlite::Result<Self> { Self::open(path) }
}
"""
    found = blocking_bridges("adversarial.rs", adversarial)
    assert found == [
        "adversarial.rs: DbHandle::ready_blocking",
        "adversarial.rs: DbHandle::query_raw_blocking",
        "adversarial.rs: DbHandle::with_reader_blocking",
        "adversarial.rs: DbReader::open_blocking",
    ], found


def test_the_predicate_leaves_the_writer_producer_alone() -> None:
    """The legitimate shape: a synchronous producer that only enqueues."""
    honest = """
impl DbWriter {
    pub fn write_blocking(&self, op: WriteOp) {}
    pub fn shutdown_blocking(&self) {}
}

impl DbHandle {
    pub async fn ready(&self) -> DbResult<()> { Ok(()) }
    pub async fn query(&self, sql: &str) -> DbResult<DbQueryJson> { Ok(Default::default()) }
}
"""
    assert blocking_bridges("honest.rs", honest) == []
