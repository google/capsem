"""Load the repository-wide pytest safety harness for build-system tests."""

from __future__ import annotations

import importlib.util
from pathlib import Path
from types import ModuleType

import pytest
from capsem_builder.cache.config import load_policy

REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
SHARED_CONFTEST = REPOSITORY_ROOT / "tests" / "conftest.py"
PLUGIN_NAME = "capsem_shared_test_harness"
CACHE_POLICY = load_policy(REPOSITORY_ROOT)


@pytest.fixture(autouse=True)
def _tests_do_not_inherit_cache_authority(monkeypatch: pytest.MonkeyPatch) -> None:
    """Keep synthetic repositories independent from their parent gate cache."""
    monkeypatch.delenv(CACHE_POLICY.authority_environment, raising=False)


def _same_file(plugin: object, path: Path) -> bool:
    candidate = getattr(plugin, "__file__", None)
    return bool(candidate) and Path(candidate).resolve() == path.resolve()


def _load_shared_plugin() -> ModuleType:
    spec = importlib.util.spec_from_file_location(PLUGIN_NAME, SHARED_CONFTEST)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load shared pytest harness from {SHARED_CONFTEST}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def pytest_configure(config: pytest.Config) -> None:
    """Register the shared harness once when root pytest did not load it."""
    if any(_same_file(plugin, SHARED_CONFTEST) for plugin in config.pluginmanager.get_plugins()):
        return
    config.pluginmanager.register(_load_shared_plugin(), PLUGIN_NAME)
