"""The gate resolves every reusable root through the cache path library."""

from __future__ import annotations

from pathlib import Path

import pytest
from capsem_builder.gate import cachelayout
from capsem_builder.gate import config as gate_config

PROJECT_ROOT = Path(__file__).resolve().parents[3]
CONFIG = gate_config.load(PROJECT_ROOT)


def test_checked_in_shared_roots_are_inside_repository_cache(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.delenv(CONFIG.environment.source_checkout, raising=False)
    resolved = {
        cachelayout.shared_path(CONFIG, value)
        for value in (
            CONFIG.prefix.parent,
            CONFIG.prefix.build_cache,
            CONFIG.prefix.vm_image_cache,
            CONFIG.prefix.cargo_target,
        )
    }

    assert resolved
    assert all(path.is_relative_to(PROJECT_ROOT / "cache") for path in resolved)


def test_private_gate_uses_outer_source_checkout_as_cache_authority(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    outer = tmp_path / "outer"
    outer.mkdir()
    monkeypatch.setenv(CONFIG.environment.source_checkout, str(outer))
    config = CONFIG

    assert cachelayout.shared_path(config, Path("cache/worktrees")) == outer / "cache/worktrees"


def test_absolute_test_override_remains_explicit(tmp_path: Path) -> None:
    override = tmp_path / "isolated"

    assert cachelayout.shared_path(CONFIG, override) == override


def test_uv_generation_is_keyed_by_the_locked_python_project(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.delenv(CONFIG.environment.source_checkout, raising=False)
    generation = cachelayout.keyed_stage_path(
        CONFIG, "python-uv", *CONFIG.toolchain.uv_identity_inputs
    )

    assert generation.parent == PROJECT_ROOT / "cache/tools/python/uv"
    assert len(generation.name) == 64
