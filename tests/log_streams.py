"""Read a rotated Capsem log stream from Python.

`<run>/service.log` names a daily-rotated stream, not a file. Opening that name
directly returns nothing once rotation has happened, so a test asserting on log
content silently asserts on an empty string.

This is the Python half of `capsem_core::telemetry::read_log_tail`. Writers
still pass the stream name -- that is what the appender expects -- but readers
resolve it here.
"""

from __future__ import annotations

from pathlib import Path


def log_stream_files(stream: Path) -> list[Path]:
    """Every file in `stream`, oldest first.

    Matches `service.log` and `service.<date>.log`, never `services.log`.
    """
    stem, suffix = stream.stem, stream.suffix.lstrip(".")
    found = [
        path
        for path in stream.parent.glob(f"{stem}*.{suffix}")
        if path.is_file() and (path.name == stream.name or path.name.startswith(f"{stem}."))
    ]
    return sorted(found, key=lambda p: p.stat().st_mtime)


def read_log_stream(stream: Path) -> str:
    """The whole stream, oldest line first, or an empty string if it has none."""
    return "".join(
        path.read_text(encoding="utf-8", errors="replace")
        for path in log_stream_files(stream)
    )
