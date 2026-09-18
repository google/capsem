"""Citadel guards for DB-boundary regressions.

The Citadel is where Capsem records architectural mistakes that must not be
repeated. These tests are intentionally source-level: they fail before a hidden
route cache, direct SQLite open, or compatibility fallback can ship green.
"""

from __future__ import annotations

import re
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
CRATES_DIR = PROJECT_ROOT / "crates"

DB_BOUNDARY_RATIONALE = """\
Logged-data DB boundary violation.

capsem-logger owns SQLite execution, storage mechanics, memory/disk tables,
batching, flushing, rehydration, WAL tuning, and future FTS/search. Service,
gateway, MCP, UI, core, and route code may own query intent, but all ledger
reads/writes must go through the DB object: db.ready().await,
db.query(sql, params).await, and db.write(event).await.

Do not add direct SQLite opens, direct DbReader construction, service-owned
route projections, missing-schema fallbacks, or route-specific DbWriter helpers.
Empty tables are valid empty results; missing tables/columns are broken schema
and must fail loudly.

See AGENTS.md and skills/dev-testing/SKILL.md section 'Logged-data DB ownership'.
"""

FORBIDDEN_PATTERNS: tuple[tuple[str, str], ...] = (
    ("rusqlite::Connection", "raw SQLite access outside the logger DB object"),
    ("Connection::open", "raw SQLite open outside the logger DB object"),
    ("Connection::open_with_flags", "raw SQLite open outside the logger DB object"),
    ("DbReader::open", "direct reader construction bypasses the DB handle"),
    ("SessionDb::new", "direct session DB construction bypasses the DB handle"),
    ("request_projection_refresh", "route/service projection cache must be burned"),
    ("route_projection", "route/service projection cache must be burned"),
    ("_route_projection", "route/service projection cache must be burned"),
    ("live_session_counter_projection", "route/service projection cache must be burned"),
    ("missing_optional_ledger_shape", "missing schema must fail loudly"),
    ("no such table", "missing schema fallback must not be special-cased"),
    ("no such column", "missing schema fallback must not be special-cased"),
)

ROUTE_HELPER_PATTERNS: tuple[str, ...] = (
    "stats_detail_payload",
    "security_route_payload",
    "history_route_payload",
    "timeline_route_payload",
    "triage_route_payload",
    "model_stats_payload",
    "tool_stats_payload",
    "http_stats_payload",
    "dns_stats_payload",
    "file_stats_payload",
    "process_stats_payload",
    "credential_stats_payload",
)

# The benchmark time series is a different database with a different schema,
# not the logged-data ledger this boundary is about. It follows the same rule
# the boundary exists to enforce -- exactly one module owns the connection, the
# schema and the queries, and nothing else opens it -- so it is named here as
# an owner rather than exempted as an exception. A benchmark harness reading
# `session.db` is still bound by the rule above: that is a ledger read and goes
# through the DB object.
BENCHMARK_DB_INTERNALS = {
    Path("crates/capsem-bench/src/store.rs"),
    Path("crates/capsem-bench/src/store/tests.rs"),
}

LOGGER_DB_INTERNALS = {
    Path("crates/capsem-logger/src/db.rs"),
    Path("crates/capsem-logger/src/db/handle_tests.rs"),
    Path("crates/capsem-logger/src/db/handle_tests/bodies.rs"),
    Path("crates/capsem-logger/src/db/handle_tests/external_reader.rs"),
    Path("crates/capsem-logger/src/db/handle_tests/query.rs"),
    Path("crates/capsem-logger/src/db/handle_tests/retention.rs"),
    Path("crates/capsem-logger/src/db/maintenance.rs"),
    Path("crates/capsem-logger/src/db/reader_worker.rs"),
    Path("crates/capsem-logger/src/network_db.rs"),
    Path("crates/capsem-logger/src/reader.rs"),
    Path("crates/capsem-logger/src/reader/open.rs"),
    Path("crates/capsem-logger/src/schema.rs"),
    Path("crates/capsem-logger/src/schema/pragmas.rs"),
    Path("crates/capsem-logger/src/schema/security_event_types.rs"),
    Path("crates/capsem-logger/src/schema/transport.rs"),
    Path("crates/capsem-logger/src/session_index.rs"),
    Path("crates/capsem-logger/src/writer.rs"),
    # The writer thread's own modules. They run on the thread that owns the
    # connection and are handed it as an argument; they do not open one.
    Path("crates/capsem-logger/src/writer/barriers.rs"),
    Path("crates/capsem-logger/src/writer/bodies.rs"),
    Path("crates/capsem-logger/src/writer/model_rows.rs"),
    Path("crates/capsem-logger/src/writer/retention.rs"),
    Path("crates/capsem-logger/src/writer/tests.rs"),
}


IMPORT_NAME = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
BRACED_IMPORT_START = "rusqlite::{"


def named_paths(source: str) -> str:
    """The source, plus every name any `rusqlite::{...}` import brings in.

    `use rusqlite::{params, Connection}` names `rusqlite::Connection` as surely
    as spelling it out does, and it is how Rust imports are ordinarily written
    once a module needs two things from a crate. Matching the literal string
    alone let two of capsem-logger's own writer modules hold a connection
    without ever being listed as owners of one -- not because anyone decided
    they could, but because of how their `use` line was formatted. A guard that
    a rustfmt-idiomatic import walks past is not guarding anything.

    The group is scanned with a brace counter rather than a regex, because a
    regex for "braces with no braces inside" does not match a nested group at
    all: `use rusqlite::{types::{Value}, Connection};` produced *nothing*, so
    the file read as though it imported nothing from rusqlite -- a stricter
    result than the plain-text match it was meant to improve on.

    Every identifier in the group is emitted, at any depth, including `types`
    and `params`. The result is matched against a fixed list of forbidden
    strings, so a name that is not on it costs nothing; missing one that is
    costs the guard.
    """
    names: list[str] = []
    index = source.find(BRACED_IMPORT_START)
    while index != -1:
        cursor = index + len(BRACED_IMPORT_START)
        depth = 1
        while cursor < len(source) and depth:
            depth += {"{": 1, "}": -1}.get(source[cursor], 0)
            cursor += 1
        if depth:
            break  # An unbalanced group: nothing to say about it.
        group = source[index + len(BRACED_IMPORT_START) : cursor - 1]
        names.extend(f"rusqlite::{name}" for name in IMPORT_NAME.findall(group))
        index = source.find(BRACED_IMPORT_START, cursor)
    return source if not names else f"{source}\n{' '.join(names)}"


def rust_sources() -> list[Path]:
    return sorted(CRATES_DIR.rglob("*.rs"))


def relative(path: Path) -> Path:
    return path.relative_to(PROJECT_ROOT)


def is_test_source(path: Path) -> bool:
    rel = relative(path)
    return (
        "tests" in rel.parts
        or "benches" in rel.parts
        or path.name == "tests.rs"
        or path.name.startswith("test_")
    )


def is_logger_db_internal(path: Path) -> bool:
    """A module that owns a database, rather than reaching into one."""
    return relative(path) in LOGGER_DB_INTERNALS | BENCHMARK_DB_INTERNALS


def test_logger_is_the_only_database_execution_boundary() -> None:
    violations: list[str] = []
    for path in rust_sources():
        if is_test_source(path) or is_logger_db_internal(path):
            continue
        source = named_paths(path.read_text())
        for needle, reason in FORBIDDEN_PATTERNS:
            if needle in source:
                violations.append(f"{relative(path)} contains `{needle}` ({reason})")

    writer_path = PROJECT_ROOT / "crates/capsem-logger/src/writer.rs"
    writer_source = writer_path.read_text()
    for needle in ROUTE_HELPER_PATTERNS:
        if needle in writer_source:
            violations.append(
                f"{relative(writer_path)} contains `{needle}` "
                "(DbWriter must not become a route/product-view helper registry)"
            )

    assert not violations, DB_BOUNDARY_RATIONALE + "\n" + "\n".join(violations)


def test_every_named_owner_actually_holds_a_connection() -> None:
    """An exemption is for a violation that exists, not a file that is nearby.

    Membership is exact-set and blanket: a path on the list is excused from
    every pattern above, not only the one it was added for. Three files were
    once added alongside two that genuinely needed it -- they only reached
    `Connection` through `use super::*` -- and each of them silently gained a
    permanent pass on `Connection::open`, `DbReader::open`, the projection
    caches and the missing-schema fallbacks.

    So the list has to earn itself: every entry must still match something.
    An entry that stops matching is not harmless, it is a carve-out with
    nothing under it, and it goes.
    """
    unnecessary: list[str] = []
    for relative_path in sorted(LOGGER_DB_INTERNALS | BENCHMARK_DB_INTERNALS):
        path = PROJECT_ROOT / relative_path
        assert path.is_file(), f"{relative_path} is named as a database owner but does not exist"
        source = named_paths(path.read_text())
        matched = [needle for needle, _ in FORBIDDEN_PATTERNS if needle in source]
        if not matched:
            unnecessary.append(str(relative_path))

    assert not unnecessary, (
        "These files are exempted from the DB boundary and do not need to be. "
        "An exemption nobody needs is a carve-out waiting to be used by accident; "
        "remove them from LOGGER_DB_INTERNALS / BENCHMARK_DB_INTERNALS:\n"
        + "\n".join(unnecessary)
    )


def test_only_one_module_owns_the_benchmark_database() -> None:
    """The boundary is a rule about ownership, not a list of exceptions.

    Naming `store.rs` as an owner is only defensible while it is the single
    place that opens that database. If a second module in the crate starts
    opening it, the pattern has been forked and this must fail.
    """
    openers = [
        relative(path)
        for path in rust_sources()
        if path.is_relative_to(CRATES_DIR / "capsem-bench")
        and "Connection::open" in path.read_text()
    ]
    assert openers == [Path("crates/capsem-bench/src/store.rs")], (
        "the benchmark database must have exactly one owning module; found "
        f"{openers}"
    )
