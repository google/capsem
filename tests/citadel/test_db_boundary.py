"""Citadel guards for DB-boundary regressions.

The Citadel is where Capsem records architectural mistakes that must not be
repeated. These tests are intentionally source-level: they fail before a hidden
route cache, direct SQLite open, or compatibility fallback can ship green.
"""

from __future__ import annotations

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
    Path("crates/capsem-logger/src/db/handle_tests/query.rs"),
    Path("crates/capsem-logger/src/reader.rs"),
    Path("crates/capsem-logger/src/schema.rs"),
    Path("crates/capsem-logger/src/session_index.rs"),
    Path("crates/capsem-logger/src/writer.rs"),
    Path("crates/capsem-logger/src/writer/tests.rs"),
}


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
        source = path.read_text()
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
