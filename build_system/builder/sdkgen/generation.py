"""Deterministic SDK generation and exact generated-source drift checks."""

from __future__ import annotations

from pathlib import Path

from .operations import read_operations
from .python import render_models
from .python_operations import render_operations
from .schema import read_schemas
from .typescript import render_models as render_typescript_models
from .typescript_operations import render_operations as render_typescript_operations
from .typescript_validation import render_validators


def python_sources(specification: Path) -> dict[str, dict[str, str]]:
    return {
        "models": render_models(read_schemas(specification)),
        "_operations": render_operations(read_operations(specification)),
    }


def typescript_sources(specification: Path) -> dict[str, dict[str, str]]:
    schemas = read_schemas(specification)
    return {"models": render_typescript_models(schemas), "validation": render_validators(schemas),
            "operations": render_typescript_operations(read_operations(specification))}


def synchronize(package: Path, sources: dict[str, dict[str, str]], *, check: bool) -> list[str]:
    """Own only generated module directories; never modify handwritten clients."""
    changed = []
    for directory, files in sorted(sources.items()):
        root = package / directory
        expected = {root / name: source for name, source in files.items()}
        suffixes = {path.suffix for path in expected}
        stale = sorted({path for suffix in suffixes for path in root.rglob(f"*{suffix}")} - expected.keys())
        for path in stale:
            changed.append(f"stale {path}")
            if not check:
                path.unlink()
        for path, source in sorted(expected.items()):
            if not path.is_file() or path.read_text() != source:
                changed.append(f"changed {path}")
                if not check:
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_text(source)
    return changed
