"""Typed parsing helpers for Docker's machine-readable storage formats."""

from __future__ import annotations

import json
import re
from datetime import UTC, datetime
from decimal import Decimal

SIZE_UNITS = {
    "B": 1,
    "KB": 1000,
    "MB": 1000**2,
    "GB": 1000**3,
    "TB": 1000**4,
    "KIB": 1024,
    "MIB": 1024**2,
    "GIB": 1024**3,
    "TIB": 1024**4,
}


def _parse_size(value: str, *, signed: bool) -> int:
    token = value.strip().split()[0].replace(",", "")
    sign = "-?" if signed else ""
    match = re.fullmatch(
        rf"({sign}[0-9]+(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?)([A-Za-z]+)", token
    )
    if match is None or match.group(2).upper() not in SIZE_UNITS:
        raise ValueError(f"unsupported Docker size: {value!r}")
    return int(Decimal(match.group(1)) * SIZE_UNITS[match.group(2).upper()])


def parse_size(value: str) -> int:
    return _parse_size(value, signed=False)


def reclaimable_size(value: str) -> int:
    return max(0, _parse_size(value, signed=True))


def timestamp(value: str) -> int:
    if not value:
        return 0
    normalized = value.removesuffix(" UTC").replace("Z", "+00:00")
    try:
        parsed = datetime.fromisoformat(normalized)
    except ValueError:
        parsed = datetime.strptime(normalized, "%Y-%m-%d %H:%M:%S %z")
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=UTC)
    return int(parsed.timestamp() * 1_000_000_000)


def json_lines(output: str) -> tuple[dict[str, object], ...]:
    rows = tuple(json.loads(line) for line in output.splitlines() if line.strip())
    if any(not isinstance(row, dict) for row in rows):
        raise ValueError("Docker JSON-lines output contains a non-object")
    return rows
