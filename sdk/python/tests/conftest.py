"""Shared pytest configuration for Python SDK tests."""

from __future__ import annotations

import os
from pathlib import Path

import pytest


def _sanitize_basetemp_namespace(namespace: str) -> str:
    """Return a filesystem-safe basetemp subdirectory namespace."""
    cleaned = "".join(
        ch if ch.isalnum() or ch in ("-", "_") else "-" for ch in namespace
    )
    cleaned = cleaned.strip("-_")
    return cleaned or "sdk-python"


def _namespaced_basetemp(
    basetemp: str | None, env: dict[str, str] | None = None
) -> str | None:
    """Give each named pytest invocation its own directory under `--basetemp`.

    The gate exports one `--basetemp` for every step, and pytest empties it
    when a session starts. Run side by side with `fast.citadel`, an unnamespaced
    suite deletes `citadel`'s `tmp_path` directories mid-test.
    """
    source = os.environ if env is None else env
    if not basetemp or source.get("PYTEST_XDIST_WORKER"):
        return basetemp
    namespace = source.get("CAPSEM_TEST_RUN_ID", "").strip() or "sdk-python"
    return str(Path(basetemp) / _sanitize_basetemp_namespace(namespace))


@pytest.hookimpl(tryfirst=True)
def pytest_configure(config: pytest.Config) -> None:
    namespaced = _namespaced_basetemp(config.option.basetemp)
    if namespaced is not None and namespaced != config.option.basetemp:
        Path(namespaced).parent.mkdir(parents=True, exist_ok=True)
        config.option.basetemp = namespaced
        # `sdkchecks.py` runs `pytest` from the package root with
        # `testpaths = ["tests"]` and no CLI path args (and lints/typechecks
        # only `source` + `tests/`), so pytest loads `tests/conftest.py` during
        # collection after `_pytest.tmpdir.pytest_configure` has already
        # constructed `config._tmp_path_factory` from `config.option.basetemp`.
        tmp_factory = getattr(config, "_tmp_path_factory", None)
        if tmp_factory is not None:
            tmp_factory._given_basetemp = Path(namespaced).resolve()
            tmp_factory._basetemp = None
