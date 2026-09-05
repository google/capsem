"""Mount the direct builder/ source as capsem_builder before an environment exists."""

from __future__ import annotations

import importlib.util
import os
import sys
from collections.abc import Sequence
from pathlib import Path


def reexec_project_python(root: Path, script: Path, argv: Sequence[str]) -> None:
    """Enter the locked builder environment before loading package dependencies."""
    interpreter = root / "build_system" / ".venv" / "bin" / "python"
    if interpreter.is_file() and Path(sys.executable).resolve() != interpreter.resolve():
        os.execv(str(interpreter), [str(interpreter), str(script), *argv])


def mount_builder_package(root: Path) -> None:
    """Expose the canonical direct source without a mirror package or symlink."""
    if importlib.util.find_spec("capsem_builder") is not None:
        return
    source = root / "build_system" / "builder"
    spec = importlib.util.spec_from_file_location(
        "capsem_builder",
        source / "__init__.py",
        submodule_search_locations=[str(source)],
    )
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot mount capsem-builder source: {source}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
