"""Release graph, source identity, and published-evidence ownership."""

from __future__ import annotations

from pathlib import Path


def project_root(module_file: str) -> Path:
    """Return the checkout that owns an editable or installed release module."""
    source_root = Path(module_file).resolve().parents[3]
    for candidate in (source_root, Path.cwd().resolve()):
        if (candidate / "justfile").is_file():
            return candidate
    raise RuntimeError(
        "capsem_builder.release must run from a Capsem checkout containing justfile"
    )
