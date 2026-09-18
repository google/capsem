"""Read the wire's encoded byte payloads.

Exec and file routes carry bytes as ``{"encoding": "utf8"|"base64", "data": ...}``
so that output which is not valid UTF-8 survives the trip. Tests that assert on
command output decode it here instead of each spelling the shape again.
"""

from __future__ import annotations

import base64
from typing import Any


def decoded_text(payload: Any) -> str:
    """The text of one encoded payload, or "" when the field is absent."""
    if payload is None:
        return ""
    if isinstance(payload, str):  # A route that still answers plain text.
        return payload
    data = payload.get("data", "")
    if payload.get("encoding") == "base64":
        return base64.b64decode(data).decode("utf-8", "replace")
    return data
