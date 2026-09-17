"""Citadel guard: the documented MCP tool surface is the registered one.

The Citadel is where Capsem records architectural mistakes that must not be
repeated. This one is documentation describing tools that do not exist.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[2]
TOOL_SOURCES = PROJECT_ROOT / "mcp/typescript/src"
#: Found by name rather than by path: a literal documentation path here would
#: be one more caller for `test_web_benchmark_boundary` to inventory.
TOOL_DOCS = sorted((PROJECT_ROOT / "web").rglob("mcp-tools.md"))

REGISTERED = re.compile(r"registerTool\(\s*'(capsem_[a-z0-9_]+)'")
TABLE_ROW_TOOLS = re.compile(r"`(capsem_[a-z0-9_]+)`")
MENTION = re.compile(r"`(capsem_[a-z0-9_]+)`")

MCP_TOOL_DOCS_RATIONALE = """\
Every tool the MCP tools page lists must be registered, and every registered
tool must be on that page.

The gateway-SDK rewrite documented `capsem_panics`, `capsem_triage` and
`capsem_changes` while registering none of them, and omitted
`capsem_file_history`, which is the tool that actually serves /changes. An
agent following the page called tools the server answered with "unknown tool",
and nothing compared the two lists. Tables are the contract: a row names a
callable tool. Prose may name retired or guest-side tools.
"""


def registered_tools(sources: dict[str, str]) -> set[str]:
    return {name for text in sources.values() for name in REGISTERED.findall(text)}


def table_tools(doc: str) -> set[str]:
    names: set[str] = set()
    for line in doc.splitlines():
        if not line.startswith("|"):
            continue
        first_cell = line.split("|")[1]
        names.update(TABLE_ROW_TOOLS.findall(first_cell))
    return names


def surface_drift(sources: dict[str, str], doc: str) -> tuple[set[str], set[str]]:
    registered = registered_tools(sources)
    unregistered = table_tools(doc) - registered
    undocumented = registered - set(MENTION.findall(doc))
    return unregistered, undocumented


def test_documented_and_registered_mcp_tools_agree() -> None:
    assert len(TOOL_DOCS) == 1, f"exactly one MCP tools page is expected: {TOOL_DOCS}"
    sources = {path.name: path.read_text() for path in TOOL_SOURCES.rglob("*.ts")}
    unregistered, undocumented = surface_drift(sources, TOOL_DOCS[0].read_text())
    assert not unregistered and not undocumented, (
        f"documented but not registered: {sorted(unregistered)}; "
        f"registered but not documented: {sorted(undocumented)}.\n\n{MCP_TOOL_DOCS_RATIONALE}"
    )


SOURCE = {"tools.ts": "server.registerTool('capsem_list', {});\nserver.registerTool(\n  'capsem_exec', {});"}


@pytest.mark.parametrize(
    ("doc", "expected"),
    [
        ("| `capsem_list` / `capsem_exec` | x |", (set(), set())),
        ("| `capsem_list` | x |\n| `capsem_ghost` | x |\nuses `capsem_exec`", ({"capsem_ghost"}, set())),
        ("| `capsem_list` | calls `capsem_exec` internally |", (set(), set())),
        ("| `capsem_list` | x |", (set(), {"capsem_exec"})),
        ("The retired `capsem_suspend` is gone. `capsem_list`, `capsem_exec`.", (set(), set())),
    ],
)
def test_drift_reads_table_rows_and_mentions(doc: str, expected: tuple[set[str], set[str]]) -> None:
    assert surface_drift(SOURCE, doc) == expected
