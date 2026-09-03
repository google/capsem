"""The gate resolves every reusable root through the cache path library."""

from __future__ import annotations

from pathlib import Path

import pytest
from capsem_builder.cache.config import load_policy
from capsem_builder.gate import cachelayout
from capsem_builder.gate import config as gate_config

PROJECT_ROOT = Path(__file__).resolve().parents[3]
CONFIG = gate_config.load(PROJECT_ROOT)
CACHE_POLICY = load_policy(PROJECT_ROOT)


def test_checked_in_shared_roots_are_inside_repository_cache(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.delenv(CACHE_POLICY.authority_environment, raising=False)
    resolved = {
        cachelayout.shared_path(CONFIG, value)
        for value in (
            CONFIG.prefix.parent,
            CONFIG.prefix.build_cache,
            CONFIG.prefix.cargo_target,
        )
    }

    assert resolved
    assert all(path.is_relative_to(PROJECT_ROOT / "cache") for path in resolved)


def test_private_gate_uses_explicit_shared_cache_authority(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    outer = tmp_path / "outer"
    outer.mkdir()
    monkeypatch.setenv(CACHE_POLICY.authority_environment, str(outer))
    config = CONFIG

    assert cachelayout.shared_path(config, Path("cache/worktrees")) == outer / "cache/worktrees"


def test_absolute_test_override_remains_explicit(tmp_path: Path) -> None:
    override = tmp_path / "isolated"

    assert cachelayout.shared_path(CONFIG, override) == override


def test_uv_uses_its_content_addressed_shared_stage(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.delenv(CACHE_POLICY.authority_environment, raising=False)

    assert cachelayout.stage_path(CONFIG, "python-uv") == PROJECT_ROOT / "cache/tools/python/uv"


def test_container_cache_paths_come_from_the_same_stage_contract() -> None:
    assert cachelayout.stage_relative_path(CONFIG, "state") == Path("cache/state")
