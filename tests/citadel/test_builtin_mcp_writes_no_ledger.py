"""Citadel guard: the builtin MCP server holds no session-ledger writer.

A session's ledger -- `session.db` and the `session.bodies` archive beside it
-- has one writer, and it is capsem-process. `capsem-mcp-builtin` opened a
second `DbWriter` on the same ledger from its first day. SQLite serialised the
two with WAL and a busy timeout, so nothing looked wrong until the body
archive arrived: an archive writer reads the file's end once and numbers every
block it seals from a private counter, while its writes land at the real end.
The first block the builtin sealed moved the real end under capsem-process,
and every body capsem-process indexed afterwards pointed at the wrong bytes --
lost, because the hash check refuses them, rather than served wrong, but lost.

The builtin now hands what it did back to capsem-process as records on its
tool results (`capsem_proto::mcp_contracts::builtin_ledger`). Depending on
`capsem-logger` is how a second writer comes back, so it may not, not even for
its tests: a test that opens a ledger is a test that assumes the builtin
writes one.
"""

from __future__ import annotations

from collections.abc import Mapping
from pathlib import Path
from typing import Any

import tomllib

PROJECT_ROOT = Path(__file__).resolve().parents[2]
BUILTIN_MANIFEST = PROJECT_ROOT / "crates" / "capsem-mcp-builtin" / "Cargo.toml"
LEDGER_CRATE = "capsem-logger"

ONE_WRITER_RATIONALE = """\
capsem-mcp-builtin must not depend on capsem-logger.

capsem-process is the one writer of a session's ledger. A second process with
a DbWriter on the same session.db also appends to session.bodies, and the
archive cannot survive two appenders: each numbers block offsets from its own
counter, so the other's blocks move them and the index points at wrong bytes.

Return what the builtin did as BuiltinLedgerRecord values on the tool result's
`_meta` (capsem_proto::mcp_contracts::builtin_ledger); capsem-process's MCP
endpoint writes them. See crates/capsem-core/src/mcp/builtin_ledger.rs.
"""


def declared_crates(manifest: Mapping[str, Any]) -> set[str]:
    """Every crate a manifest depends on, in any table, under any alias."""
    found: set[str] = set()
    for key, value in manifest.items():
        if not isinstance(value, Mapping):
            continue
        if key in {"dependencies", "dev-dependencies", "build-dependencies"}:
            for alias, declaration in value.items():
                package = (
                    declaration.get("package", alias)
                    if isinstance(declaration, Mapping)
                    else alias
                )
                found.add(str(package))
        else:
            found.update(declared_crates(value))
    return found


def test_the_builtin_server_does_not_depend_on_the_ledger_crate() -> None:
    manifest = tomllib.loads(BUILTIN_MANIFEST.read_text())
    assert LEDGER_CRATE not in declared_crates(manifest), ONE_WRITER_RATIONALE


def test_the_predicate_sees_every_way_to_declare_the_dependency() -> None:
    """A guard a renamed or target-specific dependency walks past guards nothing."""
    for source in (
        '[dependencies]\ncapsem-logger = { path = "../capsem-logger" }\n',
        '[dev-dependencies]\ncapsem-logger = { path = "../capsem-logger" }\n',
        '[dependencies]\nledger = { package = "capsem-logger", path = "../capsem-logger" }\n',
        "[target.'cfg(unix)'.dependencies]\ncapsem-logger = { path = \"../capsem-logger\" }\n",
    ):
        assert LEDGER_CRATE in declared_crates(tomllib.loads(source)), source

    unrelated = '[dependencies]\ncapsem-proto = { path = "../capsem-proto" }\n'
    assert LEDGER_CRATE not in declared_crates(tomllib.loads(unrelated))
