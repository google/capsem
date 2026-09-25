"""Session totals come from the ledger's counter snapshot, not from its rows.

Every polled number -- /vms/list, /info, stats/summary, security status,
history counts, plugin and credential runtime -- used to be a COUNT, SUM,
GROUP BY or json_each over the session ledger, run on every poll, so a poll
cost as much as the session was long. They moved to a snapshot the writer
keeps and persists beside the rows it counts (#223). These guards keep them
there.
"""

from __future__ import annotations

import hashlib
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CRATES = ROOT / "crates"

COUNTER_BOUNDARY_RATIONALE = """
Session totals are engine-owned (#223): capsem-logger's writer advances
LedgerCounters for every committed op and persists them in the
ledger_counters row, and routes read that row with one cached primary-key
lookup. An aggregate over ledger rows is O(ledger) on every call; polled
routes run it per VM on a timer. Read DbHandle::ledger_counters(); add the
count to LedgerCounters if it is missing. The SQL oracle that holds the
snapshot to the rows lives in capsem-logger/src/counters/oracle.rs, a test.
"""

# Aggregate SQL that is allowed, each one named: `path:statement-hash` ->
# why. A statement is the whole string literal it appears in, with SQL
# comments stripped and whitespace collapsed, so editing an allowed statement
# makes it a new one to justify, and adding one beside it fails.
AGGREGATE_DEBT: dict[str, str] = {
    "crates/capsem-logger/src/db/maintenance.rs:be5c29f7ac00": (
        "fork-time validation that every body index row names a block; runs once per fork, not per poll"
    ),
    "crates/capsem-logger/src/schema.rs:765473e8fa13": (
        "open-time check of whether a ledger missing its transport tables has recorded anything; stops at "
        "the first table with a row"
    ),
    "crates/capsem-logger/src/schema.rs:8cbf162ea190": "archive_state is a singleton; this proves it has one row",
    "crates/capsem-logger/src/schema/memory_sync.rs:1fb15a26fadd": (
        "the flush folds the rule matches it moves to disk into counted runs; it groups one flush's rows"
    ),
    "crates/capsem-logger/src/schema/memory_sync.rs:6493cf061729": (
        "the flush folds the decision transitions it moves to disk into counted runs; one flush's rows"
    ),
    "crates/capsem-logger/src/session_index.rs:01c7d0dfd4aa": "main.db: how many sessions it indexes",
    "crates/capsem-logger/src/writer/retention.rs:4b420a811f87": (
        "retention reports the blocks it keeps; runs on an explicit retention request, over the block index"
    ),
    "crates/capsem-service/src/ledger_routes/global_stats.rs:afb847047612": (
        "GET /stats totals across sessions from main.db, one row per session filled from each session's "
        "snapshot at stop; it never reads a session ledger"
    ),
    "crates/capsem-service/src/ledger_routes/history.rs:b803454453eb": (
        "GET /history total when a search is given: how many exec/audit rows match a literal substring. "
        "No snapshot can hold a count per search text and the ledger has no full-text index yet, so this is "
        "a scan bounded by the reader's 5-second interrupt; without a search the total is the counter snapshot"
    ),
    "crates/capsem-service/src/ledger_routes/stats_detail/interactions.rs:817eea121bc9": (
        "stats detail rows: folds each model item's tool responses into one error flag per item; a detail "
        "view over a bounded page of rows, not a polled total"
    ),
}

# Files that implement polled routes. None may carry aggregate SQL, debt or
# not: the whole point of the snapshot is that these stay flat in ledger size.
POLLED_ROUTE_FILES = {
    "crates/capsem-service/src/sandbox_info.rs": "handle_list",
    "crates/capsem-service/src/ledger_routes/vm_info.rs": "populate_vm_info",
    "crates/capsem-service/src/ledger_routes/activity.rs": "read_counters",
    "crates/capsem-service/src/ledger_routes/security.rs": "security_stats_for_vm",
    "crates/capsem-service/src/ledger_routes.rs": "hydrate_plugin_execution_runtime",
    "crates/capsem-service/src/vm_lifecycle.rs": "handle_history_counts",
    "crates/capsem-service/src/vm_files.rs": "handle_stats_summary",
    "crates/capsem-gateway/src/status.rs": "fn ",
}

# Tables whose rows the snapshot counts. Deleting one makes the snapshot
# disagree with the ledger forever: nothing recounts.
COUNTED_TABLES = (
    "net_events",
    "model_calls",
    "tool_calls",
    "fs_events",
    "exec_events",
    "audit_events",
    "substitution_events",
    "security_rule_events",
    "security_rule_runs",
    "security_ask_events",
)

# DELETE statements built from a template, which no table name can be read
# from, each one named the same way as AGGREGATE_DEBT.
TEMPLATED_DELETES: dict[str, str] = {
    "crates/capsem-logger/src/schema/memory_sync.rs:cd5d6076321f": (
        "the writer's flush empties the memory schema of rows it has just copied to disk, in the same "
        "transaction; the rows stay on disk and the snapshot keeps counting them (#213)"
    ),
}

AGGREGATE = re.compile(r"(?<![\w.])(?:count|sum)\s*\(|\bgroup\s+by\b|\bjson_each\b", re.IGNORECASE)
DELETE = re.compile(r"\bdelete\s+from\s+([^\s(;]+)", re.IGNORECASE)
LITERAL = re.compile(r'r(#*)"(.*?)"\1|"((?:\\.|[^"\\])*)"', re.DOTALL)


def is_test_source(path: Path) -> bool:
    parts = path.relative_to(ROOT).parts
    return (
        path.name == "tests.rs"
        or "tests" in parts
        or "benches" in parts
        or any(Path(part).stem.endswith("_tests") for part in parts)
        or (path.parent.name == "counters" and path.name in {"oracle.rs", "fixtures.rs", "equivalence.rs"})
    )


def production_sources() -> list[Path]:
    return sorted(path for path in CRATES.rglob("*.rs") if not is_test_source(path))


CHAR_LITERAL = re.compile(r"'(?:\\.|[^'\\])'")


def literals(source: str) -> list[str]:
    """Every string literal in a Rust source, raw or plain.

    Char literals go first: a `'"'` would otherwise open a string that runs
    to the next quote and throw off every literal after it. Comments need no
    stripping -- prose about SQL has no quotes around it.
    """
    source = CHAR_LITERAL.sub("''", source)
    return [raw if raw else plain for _, raw, plain in LITERAL.findall(source)]


def normalized(statement: str) -> str:
    without_comments = re.sub(r"--[^\n]*", "", statement)
    return " ".join(without_comments.split())


def statement_key(relative: str, statement: str) -> str:
    digest = hashlib.sha256(normalized(statement).encode()).hexdigest()[:12]
    return f"{relative}:{digest}"


def aggregate_statements(sources: list[Path]) -> dict[str, str]:
    found: dict[str, str] = {}
    for path in sources:
        relative = path.relative_to(ROOT).as_posix()
        for statement in literals(path.read_text()):
            if AGGREGATE.search(normalized(statement)):
                found[statement_key(relative, statement)] = normalized(statement)[:120]
    return found


def delete_statements(sources: list[Path]) -> tuple[list[str], dict[str, str]]:
    counted, templated = [], {}
    for path in sources:
        relative = path.relative_to(ROOT).as_posix()
        for statement in literals(path.read_text()):
            for target in DELETE.findall(normalized(statement)):
                table = target.split(".")[-1]
                if "{" in target:
                    templated[statement_key(relative, statement)] = normalized(statement)[:120]
                elif table in COUNTED_TABLES:
                    counted.append(f"{relative}: {normalized(statement)[:120]}")
    return counted, templated


def test_aggregate_sql_is_confined_to_named_debt() -> None:
    found = aggregate_statements(production_sources())
    unnamed = {key: sql for key, sql in found.items() if key not in AGGREGATE_DEBT}
    stale = sorted(set(AGGREGATE_DEBT) - set(found))
    assert not unnamed, (
        COUNTER_BOUNDARY_RATIONALE
        + "\nAggregate SQL outside the named debt list:\n"
        + "\n".join(f"  {key}  {sql}" for key, sql in sorted(unnamed.items()))
    )
    assert not stale, "AGGREGATE_DEBT names statements that no longer exist; remove them:\n" + "\n".join(stale)


def test_polled_routes_carry_no_aggregate_sql() -> None:
    for relative, anchor in POLLED_ROUTE_FILES.items():
        path = ROOT / relative
        assert path.is_file() and anchor in path.read_text(), (
            f"{relative} no longer holds {anchor!r}: update POLLED_ROUTE_FILES to where the polled route moved, "
            "or this guard stops watching it"
        )
    polled = [ROOT / relative for relative in POLLED_ROUTE_FILES]
    found = aggregate_statements(polled)
    assert not found, (
        COUNTER_BOUNDARY_RATIONALE
        + "\nA polled route file runs aggregate SQL (no debt is accepted here):\n"
        + "\n".join(f"  {key}  {sql}" for key, sql in sorted(found.items()))
    )


def test_counted_rows_are_never_deleted() -> None:
    counted, templated = delete_statements(production_sources())
    assert not counted, (
        COUNTER_BOUNDARY_RATIONALE
        + "\nA counted ledger row is deleted; the snapshot would never agree with the ledger again:\n  "
        + "\n  ".join(counted)
    )
    unnamed = {key: sql for key, sql in templated.items() if key not in TEMPLATED_DELETES}
    stale = sorted(set(TEMPLATED_DELETES) - set(templated))
    assert not unnamed, (
        "A DELETE built from a template names no table this guard can check; name it in TEMPLATED_DELETES "
        "with why it cannot delete a counted row:\n"
        + "\n".join(f"  {key}  {sql}" for key, sql in sorted(unnamed.items()))
    )
    assert not stale, "TEMPLATED_DELETES names statements that no longer exist:\n" + "\n".join(stale)


# -- adversarial: the guard must see what it exists to refuse -----------------


def test_aggregate_spellings_are_caught() -> None:
    for sql in [
        'const Q: &str = "SELECT COUNT(*) FROM net_events";',
        'const Q: &str = r#"select sum(bytes_sent) from net_events"#;',
        'const Q: &str = "SELECT count (id) FROM exec_events";',
        'let q = "SELECT exe FROM audit_events\\n GROUP   BY exe";',
        'const Q: &str = r##"SELECT key FROM model_calls, json_each(usage_details)"##;',
    ]:
        assert any(AGGREGATE.search(normalized(literal)) for literal in literals(sql)), sql


def test_rust_code_and_prose_are_not_mistaken_for_sql() -> None:
    for code in [
        "let n = rows.iter().count();",
        "let total: u64 = values.sum();",
        "pub(crate) fn count(&self) -> usize {",
        "// the old route ran COUNT(*) over net_events",
    ]:
        assert not any(AGGREGATE.search(normalized(literal)) for literal in literals(code)), code


def test_counted_deletes_and_templates_are_caught() -> None:
    counted = 'conn.execute("DELETE FROM main.net_events WHERE id < ?1", [])'
    templated = 'conn.execute(&format!("DELETE FROM {schema}.{table} WHERE id <= ?1"), [])'
    harmless = 'conn.execute("DELETE FROM body_blocks WHERE sealed_at < ?1", [])'
    for source, expect_counted, expect_templated in [
        (counted, True, False),
        (templated, False, True),
        (harmless, False, False),
    ]:
        hits = [target for literal in literals(source) for target in DELETE.findall(normalized(literal))]
        assert any(t.split(".")[-1] in COUNTED_TABLES for t in hits) == expect_counted, source
        assert any("{" in t for t in hits) == expect_templated, source


def test_a_quote_char_literal_does_not_derail_the_scan() -> None:
    source = """let quote = '"'; let q = "SELECT COUNT(*) FROM tool_calls";"""
    assert any(AGGREGATE.search(normalized(literal)) for literal in literals(source))
