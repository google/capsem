"""Citadel guard: one `transport_events` DDL, shared by session and network DBs.

A network database keeps the same audit rows as a session ledger so that a
view across networks is a union of identical tables. The moment a second
`CREATE TABLE transport_events` exists, the two shapes drift independently
and the union becomes a migration nobody scheduled. The network database
therefore defines no transport table of its own: it obtains the session
schema through the shared `DbHandle::open`, and the only DDL for the table
lives in `crates/capsem-logger/src/schema/transport.rs`.
"""

from __future__ import annotations

import re
from pathlib import Path

from citadel.test_db_boundary import PROJECT_ROOT, relative, rust_sources

TRANSPORT_DDL = re.compile(
    r"CREATE\s+TABLE(?:\s+IF\s+NOT\s+EXISTS)?\s+transport_events\b", re.IGNORECASE
)
TRANSPORT_SCHEMA = Path("crates/capsem-logger/src/schema/transport.rs")
NETWORK_DB = Path("crates/capsem-logger/src/network_db.rs")
SHARED_OPEN = "DbHandle::open("

RATIONALE = """\
The transport ledger has exactly one shape.

`transport_events` is created by crates/capsem-logger/src/schema/transport.rs
and by nothing else. A network database takes it from the shared DbHandle so
that cross-network log views are a union of identical tables, never a
migration between two definitions that drifted apart.
"""


def transport_ddl_owners(sources: dict[Path, str]) -> list[Path]:
    """Every file that defines the table, in path order."""
    return sorted(path for path, text in sources.items() if TRANSPORT_DDL.search(text))


def ddl_violations(sources: dict[Path, str]) -> list[str]:
    violations: list[str] = []
    owners = transport_ddl_owners(sources)
    if owners != [TRANSPORT_SCHEMA]:
        violations.append(
            f"transport_events DDL must exist exactly once, in {TRANSPORT_SCHEMA}; found {owners}"
        )
    network_db = sources.get(NETWORK_DB)
    if network_db is None:
        violations.append(
            f"{NETWORK_DB} is missing; the network database owner moved without this guard"
        )
    elif SHARED_OPEN not in network_db:
        violations.append(
            f"{NETWORK_DB} must obtain the session schema through {SHARED_OPEN}) rather than its own DDL"
        )
    return violations


def tree_sources() -> dict[Path, str]:
    return {relative(path): path.read_text() for path in rust_sources()}


def test_the_transport_ledger_ddl_is_defined_once_and_shared() -> None:
    violations = ddl_violations(tree_sources())
    assert not violations, RATIONALE + "\n" + "\n".join(violations)


def test_a_diverged_copy_in_the_network_database_is_refused() -> None:
    sources = tree_sources()
    sources[NETWORK_DB] += (
        '\nconst CREATE_NETWORK_TRANSPORT: &str = "CREATE TABLE IF NOT EXISTS transport_events (id INTEGER)";\n'
    )
    violations = ddl_violations(sources)
    assert any(
        "exactly once" in violation and str(NETWORK_DB) in violation
        for violation in violations
    ), violations


def test_a_network_database_that_stops_using_the_shared_handle_is_refused() -> None:
    sources = tree_sources()
    sources[NETWORK_DB] = sources[NETWORK_DB].replace(
        SHARED_OPEN, "DbHandle::open_private("
    )
    violations = ddl_violations(sources)
    assert any(SHARED_OPEN in violation for violation in violations), violations


def test_the_guard_reads_the_real_tree() -> None:
    """The guard's inputs are the crates the DB boundary already inventories,
    so a new crate is covered the day it appears."""
    owners = transport_ddl_owners(tree_sources())
    assert owners == [TRANSPORT_SCHEMA]
    assert (PROJECT_ROOT / NETWORK_DB).exists()
