"""Strict selection of source-keyed Python diagnostic caches."""

from __future__ import annotations

import shlex
from pathlib import Path

from pydantic import BaseModel, ConfigDict, model_validator

from .paths import CachePaths

PYTHONPYCACHEPREFIX = "PYTHONPYCACHEPREFIX"
PYTEST_ADDOPTS = "PYTEST_ADDOPTS"


def _without_pytest_cache(tokens: tuple[str, ...]) -> tuple[str, ...]:
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
        kept.append(token)
        index += 1
    return tuple(kept)


class PythonCacheEnvironment(BaseModel):
    """Exact pycache and pytest generations exported to one source process."""

    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    root: Path
    pycache_prefix: Path
    pytest_cache: Path
    pytest_addopts: str

    @model_validator(mode="after")
    def paths_are_contained(self) -> PythonCacheEnvironment:
        if not self.root.is_absolute():
            raise ValueError("Python cache root must be absolute")
        for selected in (self.pycache_prefix, self.pytest_cache):
            if not selected.is_absolute() or not selected.is_relative_to(self.root):
                raise ValueError(f"Python cache generation {selected} is outside {self.root}")
        if self.pycache_prefix.name != self.pytest_cache.name:
            raise ValueError("Python pycache and pytest generations must share one source key")
        return self

    def variables(self) -> dict[str, str]:
        return {
            PYTHONPYCACHEPREFIX: str(self.pycache_prefix),
            PYTEST_ADDOPTS: self.pytest_addopts,
        }


def select(
    paths: CachePaths, pycache_prefix: Path, *, inherited_addopts: str = ""
) -> PythonCacheEnvironment:
    """Bind pytest metadata to the same exact source key as CPython bytecode."""
    pycache = pycache_prefix.absolute()
    if pycache.parent != paths.stage("python-pycache"):
        raise ValueError(f"Python bytecode generation {pycache} is outside its policy stage")
    pytest_cache = paths.stage("python-pytest") / pycache.name
    inherited = _without_pytest_cache(tuple(shlex.split(inherited_addopts)))
    options = shlex.join((*inherited, "-o", f"cache_dir={pytest_cache}"))
    return PythonCacheEnvironment(
        root=paths.root,
        pycache_prefix=pycache,
        pytest_cache=pytest_cache,
        pytest_addopts=options,
    )
