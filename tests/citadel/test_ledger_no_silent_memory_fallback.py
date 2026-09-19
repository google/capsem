"""Citadel guard: no production code falls back to an in-memory ledger.

`capsem-mcp-builtin` opened its session ledger like this:

    let db = match std::env::var("CAPSEM_SESSION_DB") {
        Ok(path) => match DbWriter::open(Path::new(&path), 256) {
            Ok(writer) => Arc::new(writer),
            Err(e) => { warn!(...); Arc::new(DbWriter::open_in_memory(1)...) }
        },
        Err(_) => Arc::new(DbWriter::open_in_memory(1)...),
    };

An in-memory `DbWriter` accepts every row and has no file, so a builtin
server started without `CAPSEM_SESSION_DB`, or with one it cannot open, ran
perfectly: it answered tool calls, wrote telemetry into a database that is
discarded at exit, and said so once at warn level. The session ledger for that
VM then showed no builtin tool calls at all -- indistinguishable from a
session in which none were made. A telemetry sink that silently becomes
/dev/null is worse than one that is absent, because the absence is the thing
an investigator would notice.

A process whose job includes recording evidence does not start without
somewhere to record it. Misconfiguration is a startup error that names the
variable, not a warning nobody reads.

`DbWriter::open_in_memory` itself stays: tests across several crates build a
writer with no file on purpose, and that is exactly the use this guard
permits. What it refuses is a production path reaching for it.

See CLAUDE.md 'Logger DB Boundary' and skills/dev-session-debug.
"""

from __future__ import annotations

import re
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
CRATES_DIR = PROJECT_ROOT / "crates"

NEEDLE = "DbWriter::open_in_memory"

NO_MEMORY_FALLBACK_RATIONALE = """\
An in-memory session ledger is a test fixture, never a fallback.

`DbWriter::open_in_memory` accepts every row and keeps none past exit. A
production path that reaches for it when the real ledger is missing or
unopenable turns a misconfiguration into a session that reports no events --
which reads exactly like a session in which none happened.

Fail at startup, naming the variable or the path. Keep in-memory writers to
tests and benches.
"""

FN_RE = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+\w+")
TEST_GATE_RE = re.compile(r"cfg\(\s*(?:test\b|any\([^)]*\btest\b)")


def is_test_or_bench_surface(path: Path) -> bool:
    """True for a file that only exists in a test or bench build.

    Cargo's own layout is the rule: a `tests`/`benches` directory, or the
    sibling `tests.rs` this repository requires test functions to live in
    (see CLAUDE.md 'Code Style').
    """
    parts = path.parts
    if "tests" in parts or "benches" in parts:
        return True
    return path.name == "tests.rs" or path.name.endswith("_tests.rs")


def enclosing_item_is_test_gated(lines: list[str], index: int) -> bool:
    """Whether the `fn` containing line `index` carries a `#[cfg(test)]`.

    Walks back to the nearest function declaration, then reads the attribute
    and doc-comment block directly above it. Textual by necessity -- the guard
    has no compiler -- and deliberately strict: an ungated helper that a
    `#[cfg(test)]` module happens to contain reads as a violation, which is the
    safe direction for a rule about production code.
    """
    for cursor in range(index, -1, -1):
        if not FN_RE.match(lines[cursor]):
            continue
        above = cursor - 1
        while above >= 0:
            stripped = lines[above].strip()
            if not (stripped.startswith(("#[", "//")) or stripped == ""):
                break
            if TEST_GATE_RE.search(stripped):
                return True
            above -= 1
        return False
    return False


def code_of(line: str) -> str:
    """The line with any `//` tail removed.

    A guard about what production code *does* must not read prose about what it
    used to do: the doc comment on `open_session_ledger` explains the fallback
    it replaced, and naming the call is how that explanation is useful. Only
    the tail is dropped, so a call with a trailing comment is still a call.
    """
    return line.split("//", 1)[0]


def memory_fallbacks(path: Path, text: str) -> list[str]:
    """Pure predicate over (path, text): production uses of an in-memory writer."""
    if is_test_or_bench_surface(path):
        return []
    try:
        relative = path.relative_to(PROJECT_ROOT)
    except ValueError:
        relative = path
    lines = text.splitlines()
    return [
        f"{relative}:{number + 1} calls `{NEEDLE}` outside a #[cfg(test)] item"
        for number, line in enumerate(lines)
        if NEEDLE in code_of(line) and not enclosing_item_is_test_gated(lines, number)
    ]


def test_the_in_memory_writer_still_exists() -> None:
    """A guard over a call nobody can make asserts nothing."""
    writer = PROJECT_ROOT / "crates/capsem-logger/src/writer.rs"
    assert "fn open_in_memory" in writer.read_text(), f"{writer} no longer defines it; this guard is vacuous"


def test_no_production_path_falls_back_to_an_in_memory_ledger() -> None:
    violations: list[str] = []
    for path in sorted(CRATES_DIR.rglob("*.rs")):
        violations.extend(memory_fallbacks(path, path.read_text()))

    assert not violations, NO_MEMORY_FALLBACK_RATIONALE + "\n" + "\n".join(violations)


def test_the_predicate_flags_the_fallback_that_was_burned() -> None:
    """The adversarial case: the exact shape `capsem-mcp-builtin` shipped."""
    offender = PROJECT_ROOT / "crates" / "capsem-mcp-builtin" / "src" / "main.rs"
    source = """
async fn main() -> Result<()> {
    let db = match std::env::var("CAPSEM_SESSION_DB") {
        Ok(path) => match DbWriter::open(Path::new(&path), 256) {
            Ok(writer) => Arc::new(writer),
            Err(e) => {
                tracing::warn!(error = %e, "failed to open session DB, telemetry disabled");
                Arc::new(DbWriter::open_in_memory(1).expect("in-memory DB"))
            }
        },
        Err(_) => Arc::new(DbWriter::open_in_memory(1).expect("in-memory DB")),
    };
    Ok(())
}
"""
    found = memory_fallbacks(offender, source)
    assert len(found) == 2, found


def test_the_predicate_allows_a_test_fixture() -> None:
    """The legitimate shape: a gated item, or a file only tests compile."""
    gated = """
impl DbHandle {
    #[cfg(test)]
    pub(crate) fn open_existing_for_tests(path: &Path) -> rusqlite::Result<Self> {
        let writer = Arc::new(DbWriter::open_in_memory(1)?);
        Self::open_with_writer(path.to_path_buf(), writer)
    }
}
"""
    assert memory_fallbacks(PROJECT_ROOT / "crates" / "capsem-logger" / "src" / "db.rs", gated) == []

    in_a_test_file = "fn fixture() -> Arc<DbWriter> { Arc::new(DbWriter::open_in_memory(8).unwrap()) }"
    assert memory_fallbacks(PROJECT_ROOT / "crates" / "capsem-core" / "src" / "tests.rs", in_a_test_file) == []

    prose = """
/// This used to fall back to `DbWriter::open_in_memory` when the variable was
/// unset. It no longer does.
fn open_session_ledger(configured: Option<String>) -> Result<DbWriter> {
    DbWriter::open(Path::new(&configured.ok_or_else(missing)?), 256)
}
"""
    assert memory_fallbacks(PROJECT_ROOT / "crates" / "a" / "src" / "main.rs", prose) == []


def test_a_call_hiding_behind_a_trailing_comment_is_still_a_call() -> None:
    """Dropping the `//` tail must not become a way to smuggle the call in."""
    smuggled = "fn start() { let db = DbWriter::open_in_memory(1).unwrap(); // just for now\n}"
    assert len(memory_fallbacks(PROJECT_ROOT / "crates" / "a" / "src" / "main.rs", smuggled)) == 1
