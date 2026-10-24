"""Read the product update audit once, optionally within a caller's byte bound."""

from __future__ import annotations

import json
from pathlib import Path
from typing import cast


class TransitionEvidenceError(RuntimeError):
    """The update audit did not prove the requested exact transition."""


def load_update_audit(
    path: Path, *, after_line: int = 0, max_bytes: int | None = None
) -> list[dict[str, object]]:
    if after_line < 0:
        raise TransitionEvidenceError("update audit line marker must not be negative")
    if max_bytes is not None and max_bytes <= 0:
        raise TransitionEvidenceError("update audit byte budget must be positive")
    try:
        with path.open("rb") as stream:
            payload = stream.read() if max_bytes is None else stream.read(max_bytes + 1)
        if max_bytes is not None and len(payload) > max_bytes:
            raise TransitionEvidenceError("update audit exceeds configured read budget")
        lines = payload.decode("utf-8").splitlines()
    except FileNotFoundError:
        return []
    except (OSError, UnicodeError) as error:
        raise TransitionEvidenceError(f"cannot read update audit {path}: {error}") from error
    rows: list[dict[str, object]] = []
    for line_number, line in enumerate(lines[after_line:], start=after_line + 1):
        if not line.strip():
            continue
        try:
            row = json.loads(line)
        except json.JSONDecodeError as error:
            raise TransitionEvidenceError(
                f"update audit {path}:{line_number} is invalid JSON: {error}"
            ) from error
        if not isinstance(row, dict):
            raise TransitionEvidenceError(
                f"update audit {path}:{line_number} must contain an object"
            )
        rows.append(cast(dict[str, object], row))
    return rows
