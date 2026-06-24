"""Service route handlers must not open SQLite on request paths."""

from __future__ import annotations

import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SERVICE_MAIN = ROOT / "crates" / "capsem-service" / "src" / "main.rs"

FORBIDDEN_ROUTE_SQLITE = (
    "capsem_logger::DbReader::open",
    "capsem_logger::DbWriter::open",
    "capsem_core::session::SessionIndex::open",
    "rusqlite",
    "Connection::open",
    ".query_raw(",
)


def _function_body(source: str, start: int) -> str:
    brace_start = source.find("{", start)
    assert brace_start != -1, "function has no body"
    depth = 0
    for index in range(brace_start, len(source)):
        char = source[index]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return source[brace_start : index + 1]
    raise AssertionError("function body did not close")


def test_service_route_handlers_do_not_open_sqlite() -> None:
    """Routes serve memory projections; SQLite belongs to load/rebuild code."""
    source = SERVICE_MAIN.read_text()
    functions: dict[str, str] = {}
    for match in re.finditer(r"(?:async\s+)?fn\s+([a-zA-Z0-9_]+)\s*\(", source):
        functions[match.group(1)] = _function_body(source, match.start())

    sqlite_functions = {
        name
        for name, body in functions.items()
        if any(token in body for token in FORBIDDEN_ROUTE_SQLITE)
    }

    callees: dict[str, set[str]] = {}
    for name, body in functions.items():
        callees[name] = {
            candidate
            for candidate in functions
            if candidate != name and re.search(rf"\b{re.escape(candidate)}\s*\(", body)
        }

    def reaches_sqlite(name: str, seen: set[str] | None = None) -> str | None:
        seen = set() if seen is None else seen
        if name in seen:
            return None
        seen.add(name)
        if name in sqlite_functions:
            return name
        for callee in sorted(callees.get(name, set())):
            found = reaches_sqlite(callee, seen)
            if found is not None:
                return found
        return None

    offenders: list[str] = []
    for name in sorted(functions):
        if not name.startswith("handle_"):
            continue
        body = functions[name]
        for token in FORBIDDEN_ROUTE_SQLITE:
            if token in body:
                offenders.append(f"{name}: {token}")
        reached = reaches_sqlite(name)
        if reached is not None and reached != name:
            offenders.append(f"{name}: calls SQLite helper {reached}")

    assert offenders == []
