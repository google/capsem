"""Every SQL statement the service runs is registered with the route plan test.

#223 found polled routes whose cost grew with the session: `/history` loaded
every row to page in memory, `/timeline` read the oldest 10,000 rows of the
whole ledger and filtered afterwards, the stats tool list sorted every tool
call to keep 200. Each looked fine on a small ledger, which is every ledger a
test or a reviewer sees.

`crates/capsem-service/src/tests/route_query_plans.rs` holds the registry of
route statements. For each one it reads the plan (no whole-table sort, the
index probes it depends on) and runs it on two ledgers four times apart in
size: a window costs the same on both, a scan does not. A statement is only
held to that if the registry names it, so this guard closes the loop: every
source item in capsem-service that carries SQL must be reachable from the
registry, directly or through the items that build it.
"""

from __future__ import annotations

import re
from pathlib import Path

from citadel.test_ledger_counter_boundary import CHAR_LITERAL, LITERAL, ROOT, is_test_source, literals

ROUTE_STATEMENTS_RATIONALE = """
A route statement the plan registry does not reach is one nobody measures.
The ledger routes are polled for the life of a session; a statement that
scans or sorts a whole table costs a little more on every poll, and no test
ledger is large enough to show it (#223). Register the statement in
`route_statements()` in crates/capsem-service/src/tests/route_query_plans.rs:
name the const or fn that builds it, the index probes its plan relies on,
and, only if it truly must grow with the ledger, the reason in `.scanning()`.
"""

SERVICE = ROOT / "crates" / "capsem-service" / "src"
REGISTRY = SERVICE / "tests" / "route_query_plans.rs"

# SQL items that are not ledger reads, keyed `path:item`, with the reason.
NOT_LEDGER_READS: dict[str, str] = {
    "crates/capsem-service/src/ledger_routes/global_stats.rs:STATS_RESPONSE_SQL": (
        "GET /stats reads main.db, one row per session (at most about a thousand per machine), "
        "filled from each session's counter snapshot at stop; no session ledger is read"
    ),
}

SQL = re.compile(
    r"\bselect\b[\s\S]*\bfrom\b|\binsert\s+into\b|\bupdate\s+\w+\s+set\b|\bdelete\s+from\b|\bpragma\s+\w",
    re.IGNORECASE,
)
ITEM = re.compile(
    r"^[ \t]*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:const|static|fn)\s+([A-Za-z_]\w*)",
    re.MULTILINE,
)
IDENTIFIER = re.compile(r"\b[A-Za-z_]\w*\b")
LINE_COMMENT = re.compile(r"//[^\n]*")
FORMAT_CAPTURE = re.compile(r"\{([A-Za-z_]\w*)")


def names(code: str) -> set[str]:
    """The identifiers code refers to.

    String literals are not code: SQL spells `run`, `event` and `exec`, and
    reading those as references once tied a statement to half the service.
    A `format!` capture (`{ARM}`) inside one is a reference, and is kept.
    """
    code = LINE_COMMENT.sub("", CHAR_LITERAL.sub("''", code))
    captured = {name for literal in LITERAL.finditer(code) for name in FORMAT_CAPTURE.findall(literal.group(0))}
    return set(IDENTIFIER.findall(LITERAL.sub('""', code))) | captured


def items(source: str) -> list[tuple[str, str]]:
    """Each const, static or fn in a source with the text up to the next one."""
    marks = list(ITEM.finditer(source))
    return [
        (mark.group(1), source[mark.start() : marks[index + 1].start() if index + 1 < len(marks) else len(source)])
        for index, mark in enumerate(marks)
    ]


def sql_items(sources: dict[str, str]) -> dict[str, str]:
    """`path:item` for every item whose own text holds a SQL literal."""
    found: dict[str, str] = {}
    for relative, source in sources.items():
        for name, body in items(source):
            for literal in literals(body):
                if SQL.search(literal):
                    found[f"{relative}:{name}"] = " ".join(literal.split())[:100]
                    break
    return found


def registry_body(registry: str) -> str:
    """The text of `fn route_statements()`, comments removed.

    Only what that function names is run through the plan check. The rest of
    the test file reaches the whole router (`build_service_router` names
    every handler), and counting it once made every statement look covered.
    Names in comments do not count either: prose is not a statement the test
    runs.
    """
    start = registry.index("fn route_statements()")
    end = registry.find("\n}\n", start)
    return registry[start : end if end != -1 else len(registry)]


USE_CRATE = re.compile(r"^use crate::[^;]*;", re.MULTILINE | re.DOTALL)


def reachable(sources: dict[str, str], registry: str) -> set[str]:
    """`path:item` for every item `route_statements()` names, and what those build from.

    A root is a name `route_statements()` uses that the registry imports with
    `use crate::...`: an explicit import is the one reference that cannot be a
    local variable, a method, or an unrelated item sharing a common name.
    From a root, references are followed within its own file only -- a
    statement is built from consts and helpers beside it, and following
    names across files reached `run` and `default` and with them most of the
    service.
    """
    imported = {name for statement in USE_CRATE.findall(registry) for name in IDENTIFIER.findall(statement)}
    per_file = {relative: dict(items(source)) for relative, source in sources.items()}
    seen: set[str] = set()
    frontier = [
        (relative, name)
        for name in names(registry_body(registry)) & imported
        for relative, file_items in per_file.items()
        if name in file_items
    ]
    while frontier:
        relative, name = frontier.pop()
        key = f"{relative}:{name}"
        if key in seen:
            continue
        seen.add(key)
        file_items = per_file[relative]
        frontier += [(relative, other) for other in names(file_items[name]) - {name} if other in file_items]
    return seen


def unregistered(sources: dict[str, str], registry: str) -> dict[str, str]:
    covered = reachable(sources, registry)
    return {
        key: sql
        for key, sql in sql_items(sources).items()
        if key not in covered and key not in NOT_LEDGER_READS
    }


def service_sources() -> dict[str, str]:
    return {
        path.relative_to(ROOT).as_posix(): path.read_text()
        for path in sorted(SERVICE.rglob("*.rs"))
        if not is_test_source(path)
    }


def test_every_service_statement_is_registered_with_the_plan_test() -> None:
    missing = unregistered(service_sources(), REGISTRY.read_text())
    assert not missing, (
        "SQL the route plan registry does not reach:\n"
        + "\n".join(f"  {key}: {sql}" for key, sql in sorted(missing.items()))
        + ROUTE_STATEMENTS_RATIONALE
    )


def test_exemptions_name_live_sql_items() -> None:
    stale = sorted(set(NOT_LEDGER_READS) - set(sql_items(service_sources())))
    assert not stale, f"NOT_LEDGER_READS names items that no longer carry SQL: {stale}"


def test_the_registry_runs_every_statement_it_lists() -> None:
    registry = REGISTRY.read_text()
    assert "fn route_statements()" in registry
    assert "for statement in route_statements()" in registry, (
        "route_query_plans.rs must iterate the whole registry, or registering a statement proves nothing"
    )


# -- adversarial cases: the scan must not be fooled -------------------------

REGISTRY_SNIPPET = """
use crate::x::{built_sql, LISTED_SQL};

fn route_statements() -> Vec<RouteStatement> {
    vec![RouteStatement::window("a", LISTED_SQL, vec![]), RouteStatement::window("b", built_sql(), vec![])]
}
"""


def test_an_unregistered_const_is_caught() -> None:
    sources = {"x.rs": 'const LISTED_SQL: &str = "SELECT a FROM t";\nconst NEW_SQL: &str = "SELECT b FROM t";\n'}
    assert set(unregistered(sources, REGISTRY_SNIPPET)) == {"x.rs:NEW_SQL"}


def test_sql_inlined_in_a_handler_is_caught() -> None:
    sources = {"x.rs": 'async fn handle() {\n    db.query("select id from net_events order by id", &[]);\n}\n'}
    assert set(unregistered(sources, REGISTRY_SNIPPET)) == {"x.rs:handle"}


def test_raw_strings_and_writes_are_caught() -> None:
    sources = {
        "x.rs": 'const A: &str = r#"\nSELECT "q" FROM t\n"#;\nfn w() { run("DELETE FROM t"); }\n'
        'fn p() { run("PRAGMA wal_checkpoint"); }\n'
    }
    assert set(unregistered(sources, REGISTRY_SNIPPET)) == {"x.rs:A", "x.rs:w", "x.rs:p"}


def test_items_reached_through_a_builder_are_covered() -> None:
    sources = {
        "x.rs": 'fn built_sql() -> String { format!("{ARM} LIMIT 1") }\n'
        'const ARM: &str = "SELECT a FROM t";\n'
        'const OTHER: &str = "SELECT b FROM t";\n'
    }
    assert set(unregistered(sources, REGISTRY_SNIPPET)) == {"x.rs:OTHER"}


def test_a_name_in_a_registry_comment_does_not_register_it() -> None:
    sources = {"x.rs": 'const NEW_SQL: &str = "SELECT b FROM t";\n'}
    registry = REGISTRY_SNIPPET.replace("vec![Route", "// NEW_SQL is covered elsewhere\n    vec![Route")
    assert set(unregistered(sources, registry)) == {"x.rs:NEW_SQL"}


def test_a_name_used_elsewhere_in_the_test_file_does_not_register_it() -> None:
    """The plan test's other tests drive the router, which reaches every handler."""
    sources = {
        "x.rs": 'fn build_service_router() { handle(); }\n'
        'async fn handle() { db.query("SELECT id FROM net_events", &[]); }\n'
    }
    registry = REGISTRY_SNIPPET + "\n#[tokio::test]\nasync fn other() { build_service_router(); }\n"
    assert set(unregistered(sources, registry)) == {"x.rs:handle"}


def test_prose_is_not_mistaken_for_sql() -> None:
    sources = {"x.rs": 'fn msg() { error!("pick a profile"); bail!("no rows selected"); }\n'}
    assert unregistered(sources, REGISTRY_SNIPPET) == {}


def test_a_local_name_matching_an_item_does_not_register_it() -> None:
    """`page`, `name` and `window` are locals in the registry, and items elsewhere."""
    sources = {"x.rs": 'fn window() { db.query("SELECT id FROM net_events", &[]); }\n'}
    registry = REGISTRY_SNIPPET.replace("vec![Route", "let window = 1;\n    vec![Route")
    assert set(unregistered(sources, registry)) == {"x.rs:window"}


def test_coverage_does_not_spread_across_files() -> None:
    sources = {
        "x.rs": 'fn built_sql() -> String { run() }\nconst LISTED_SQL: &str = "SELECT a FROM t";\n',
        "y.rs": 'fn run() { db.query("SELECT id FROM net_events", &[]); }\n',
    }
    assert set(unregistered(sources, REGISTRY_SNIPPET)) == {"y.rs:run"}
