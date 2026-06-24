"""Service route handlers must not open SQLite on request paths."""

from __future__ import annotations

import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SERVICE_MAIN = ROOT / "crates" / "capsem-service" / "src" / "main.rs"

FORBIDDEN_ROUTE_SQLITE = (
    "capsem_logger::DbReader::open",
    "capsem_core::session::SessionIndex::open",
    ".query_raw(",
    ".join(\"session.db\")",
    ".join(\"main.db\")",
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
    offenders: list[str] = []
    for match in re.finditer(r"async\s+fn\s+(handle_[a-zA-Z0-9_]+)\s*\(", source):
        name = match.group(1)
        body = _function_body(source, match.start())
        for token in FORBIDDEN_ROUTE_SQLITE:
            if token in body:
                offenders.append(f"{name}: {token}")

    assert offenders == []

