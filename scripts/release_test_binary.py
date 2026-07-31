"""Resolve host executables for local and release functional tests.

Local qualification owns source builds. Release qualification owns no hidden
binary build: it must execute the package bytes staged from the selected
manifest, even when checkout mtimes are newer than immutable package mtimes.
"""

from __future__ import annotations

import os
import subprocess
from collections.abc import Callable, Iterable, Mapping, Sequence
from pathlib import Path

Runner = Callable[..., subprocess.CompletedProcess[str]]


def ensure_host_test_binary(
    binary: Path,
    *,
    source_paths: Iterable[Path],
    build_command: Sequence[str],
    project_root: Path,
    env: Mapping[str, str] = os.environ,
    runner: Runner = subprocess.run,
    timeout: int = 120,
) -> None:
    """Make one test executable available without crossing release ownership."""
    release_inputs = env.get("CAPSEM_RELEASE_INPUT_DIR", "").strip()
    if release_inputs:
        if not binary.is_file():
            raise FileNotFoundError(
                "manifest-selected package lacks required functional-test binary: "
                f"{binary}"
            )
        if not os.access(binary, os.X_OK):
            raise PermissionError(
                "manifest-selected package binary is not executable: "
                f"{binary}"
            )
        return

    sources = tuple(source_paths)
    if binary.is_file():
        binary_mtime = binary.stat().st_mtime
        if all(binary_mtime >= source.stat().st_mtime for source in sources):
            return

    runner(
        tuple(build_command),
        cwd=project_root,
        check=True,
        timeout=timeout,
    )
    if not binary.is_file():
        raise FileNotFoundError(f"local build did not create required binary: {binary}")
    if not os.access(binary, os.X_OK):
        raise PermissionError(f"local build created a non-executable binary: {binary}")
