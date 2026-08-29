"""Safe path primitives shared by release-input consumers."""

from __future__ import annotations

from pathlib import Path, PurePosixPath


def safe_component(value: object, label: str) -> str:
    if (
        not isinstance(value, str)
        or not value
        or value in {".", ".."}
        or "/" in value
        or "\\" in value
    ):
        raise ValueError(f"unsafe {label}: {value!r}")
    return value


def safe_relative(value: object, label: str = "release input path") -> Path:
    if not isinstance(value, str) or not value:
        raise ValueError(f"{label} is invalid")
    relative = PurePosixPath(value)
    if relative.is_absolute() or any(part in {"", ".", ".."} for part in relative.parts):
        raise ValueError(f"{label} is unsafe: {value!r}")
    return Path(*relative.parts)
