"""Private gate checkouts use exact source bytecode in shared cache storage."""

import os
import sys
from pathlib import Path
from typing import NoReturn

import pytest


def _source(root: Path, value: str) -> None:
    root.mkdir()
    root.joinpath("probe.py").write_text(f'VALUE = "{value}"\n', encoding="utf-8")


def test_launcher_reexecs_when_marker_belongs_to_other_source(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    """A private exact-commit checkout must not trust its parent's generation."""
    import capsem_builder.gatelaunch as launcher

    parent = tmp_path / "parent"
    child = tmp_path / "child"
    _source(parent, "parent")
    _source(child, "child")
    parent_environment = launcher.isolated_environment(parent)
    for name, value in parent_environment.items():
        monkeypatch.setenv(name, value)
    monkeypatch.setattr(sys, "pycache_prefix", parent_environment[launcher.PYCACHE])
    issued: list[list[str]] = []

    class Replaced(BaseException):
        """A real `execv` never returns."""

    def replace(_program: str, argv: list[str]) -> NoReturn:
        issued.append(list(argv))
        raise Replaced

    monkeypatch.setattr(os, "execv", replace)
    monkeypatch.setattr(sys, "argv", ["capsem-gate", "--help"])
    monkeypatch.setattr(launcher, "checkout", lambda: child)

    with pytest.raises(Replaced):
        launcher.main()

    assert issued
    assert os.environ[launcher.MARKER] != parent_environment[launcher.MARKER]


def test_private_source_generation_uses_outer_cache_authority(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    from capsem_builder.gatelaunch import PYCACHE, isolated_environment

    source = tmp_path / "source"
    authority = tmp_path / "authority"
    _source(source, "source")
    config = source / "config"
    config.mkdir()
    config.joinpath("cache.toml").write_text(
        'root = "cache"\n[stages.python-pycache]\npath = "tools/python/pycache"\n',
        encoding="utf-8",
    )
    config.joinpath("gate.toml").write_text(
        '[environment]\nsource_checkout = "CAPSEM_TEST_CACHE_AUTHORITY"\n',
        encoding="utf-8",
    )
    monkeypatch.setenv("CAPSEM_TEST_CACHE_AUTHORITY", str(authority))

    generation = Path(isolated_environment(source)[PYCACHE])

    assert generation.parent == authority / "cache/tools/python/pycache"
