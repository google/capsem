"""Citadel guard: published JavaScript declares the runtime it needs.

The Citadel is where Capsem records architectural mistakes that must not be
repeated. This one is a runtime API used above the floor the packages admit.
"""

from __future__ import annotations

import json
import re
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[2]

#: Every npm package Capsem publishes.
PUBLISHED = ("sdk/typescript", "mcp/typescript")
#: The oldest Node major the SDK and MCP server support.
NODE_FLOOR = 20
#: Source trees that run in the desktop app's WKWebView or in Node.
RUNTIME_SOURCES = ("sdk/typescript/src", "mcp/typescript/src", "web/app/src")
#: The one place allowed to call `AbortSignal.any`, behind a feature check.
SIGNAL_LINKER = "sdk/typescript/src/transport.ts"

RUNTIME_FLOOR_RATIONALE = """\
Published JavaScript must declare, and stay inside, its runtime floor.

The gateway SDK called `AbortSignal.any` on every request. That API arrived in
WebKit 17.4 (macOS 14.4) and Node 20.3, while the desktop app declares macOS
14.0 and neither npm package declared an engine at all. On macOS 14.0-14.3
every SDK call threw a TypeError that the app reported as "gateway offline";
the npm MCP server installed cleanly on Node 18 and failed every tool call.

So each published package states `engines.node` (npm warns before install
instead of the first tool call failing), and `AbortSignal.any` is reached only
through the transport's feature-checked linker, which falls back to manual
signal linking on older engines.
"""


ABORT_SIGNAL_ANY_CALL = re.compile(r"AbortSignal\s*\.\s*any\s*\(")
FEATURE_CHECK = re.compile(r"typeof\s+AbortSignal\s*\.\s*any\s*===\s*'function'")


def node_floor(spec: str) -> int | None:
    """The major a `>=N...` engine range admits, or None for anything looser."""
    match = re.fullmatch(r">=\s*(\d+)(?:\.\d+){0,2}", spec.strip())
    return int(match.group(1)) if match else None


@pytest.mark.parametrize("package", PUBLISHED)
def test_published_packages_declare_the_node_floor(package: str) -> None:
    manifest = json.loads((PROJECT_ROOT / package / "package.json").read_text())
    spec = manifest.get("engines", {}).get("node", "")
    floor = node_floor(spec)
    assert floor is not None and floor >= NODE_FLOOR, (
        f"{package}/package.json engines.node is {spec!r}; declare '>={NODE_FLOOR}' "
        f"or higher.\n\n{RUNTIME_FLOOR_RATIONALE}"
    )


def test_abort_signal_any_is_reached_only_through_the_feature_checked_linker() -> None:
    offenders = []
    for root in RUNTIME_SOURCES:
        for path in sorted((PROJECT_ROOT / root).rglob("*")):
            if path.suffix not in {".ts", ".js", ".mjs", ".svelte"}:
                continue
            relative = path.relative_to(PROJECT_ROOT).as_posix()
            text = path.read_text()
            calls = len(ABORT_SIGNAL_ANY_CALL.findall(text))
            guarded = relative == SIGNAL_LINKER and FEATURE_CHECK.search(text)
            if calls > (1 if guarded else 0):
                offenders.append(relative)
    assert not offenders, f"unguarded AbortSignal.any in {offenders}.\n\n{RUNTIME_FLOOR_RATIONALE}"


@pytest.mark.parametrize(
    ("source", "offends"),
    [
        ("const signal = AbortSignal.any(signals);", True),
        ("AbortSignal .any ([a, b])", True),
        ("if (typeof AbortSignal.any === 'function') return AbortSignal.any(signals);", False),
        ("// AbortSignal.any is newer than macOS 14.0", False),
    ],
)
def test_the_call_pattern_sees_calls_but_not_prose(source: str, offends: bool) -> None:
    calls = len(ABORT_SIGNAL_ANY_CALL.findall(source))
    assert (calls > (1 if FEATURE_CHECK.search(source) else 0)) is offends


@pytest.mark.parametrize(
    ("spec", "expected"),
    [
        (">=20", 20),
        (">=20.3.0", 20),
        (">= 22.1", 22),
        (">=18", 18),
        ("", None),
        ("*", None),
        ("^20", None),
        ("20.x", None),
        (">=20 || >=16", None),
    ],
)
def test_node_floor_reads_only_a_plain_lower_bound(spec: str, expected: int | None) -> None:
    assert node_floor(spec) == expected
