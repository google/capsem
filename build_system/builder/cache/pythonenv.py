"""Strict selection of source-keyed Python diagnostic caches."""

from __future__ import annotations

import os
import shlex
from pathlib import Path

from pydantic import BaseModel, ConfigDict, model_validator

from .paths import CachePaths

PYTHONPYCACHEPREFIX = "PYTHONPYCACHEPREFIX"
PYTEST_ADDOPTS = "PYTEST_ADDOPTS"
TMPDIR = "TMPDIR"


def _without_owned_pytest_storage(tokens: tuple[str, ...]) -> tuple[str, ...]:
    kept: list[str] = []
    index = 0
    while index < len(tokens):
        token = tokens[index]
        if (
            token in {"-o", "--override-ini"}
            and index + 1 < len(tokens)
            and tokens[index + 1].startswith("cache_dir=")
        ):
            index += 2
            continue
        if token.startswith(("-o=cache_dir=", "--override-ini=cache_dir=")):
            index += 1
            continue
        if token == "--basetemp" and index + 1 < len(tokens):
            index += 2
            continue
        if token.startswith("--basetemp="):
            index += 1
            continue
        kept.append(token)
        index += 1
    return tuple(kept)


class PythonCacheEnvironment(BaseModel):
    """Exact pycache and pytest generations exported to one source process."""

    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    root: Path
    pycache_prefix: Path
    pytest_cache: Path
    test_tmp: Path
    pytest_basetemp: Path
    pytest_addopts: str

    @model_validator(mode="after")
    def paths_are_contained(self) -> PythonCacheEnvironment:
        if not self.root.is_absolute():
            raise ValueError("Python cache root must be absolute")
        for selected in (self.pycache_prefix, self.pytest_cache):
            if not selected.is_absolute() or not selected.is_relative_to(self.root):
                raise ValueError(f"Python cache generation {selected} is outside {self.root}")
        if not self.test_tmp.is_absolute() or not self.pytest_basetemp.is_absolute():
            raise ValueError("test temp paths must be absolute")
        if self.pycache_prefix.name != self.pytest_cache.name:
            raise ValueError("Python pycache and pytest generations must share one source key")
        if self.pytest_basetemp.parent != self.test_tmp:
            raise ValueError("pytest basetemp must be inside the process test temp directory")
        return self

    def variables(self) -> dict[str, str]:
        return {
            PYTHONPYCACHEPREFIX: str(self.pycache_prefix),
            PYTEST_ADDOPTS: self.pytest_addopts,
            TMPDIR: str(self.test_tmp),
        }


def select(
    paths: CachePaths,
    pycache_prefix: Path,
    *,
    inherited_addopts: str = "",
    process_id: int | None = None,
) -> PythonCacheEnvironment:
    """Bind reusable Python state and ephemeral test work to owned stages."""
    pycache = pycache_prefix.absolute()
    if pycache.parent != paths.stage("python-pycache"):
        raise ValueError(f"Python bytecode generation {pycache} is outside its policy stage")
    selected_pid = os.getpid() if process_id is None else process_id
    if selected_pid <= 0:
        raise ValueError("test temp process ID must be positive")
    pytest_cache = paths.stage("python-pytest") / pycache.name
    test_tmp = paths.stage("test-temp") / f"run-{selected_pid}"
    pytest_basetemp = test_tmp / "pytest"
    inherited = _without_owned_pytest_storage(tuple(shlex.split(inherited_addopts)))
    options = shlex.join(
        (*inherited, "-o", f"cache_dir={pytest_cache}", f"--basetemp={pytest_basetemp}")
    )
    return PythonCacheEnvironment(
        root=paths.root,
        pycache_prefix=pycache,
        pytest_cache=pytest_cache,
        test_tmp=test_tmp,
        pytest_basetemp=pytest_basetemp,
        pytest_addopts=options,
    )
