"""Path resolution must keep every persistent byte inside repository cache/."""

from pathlib import Path

import pytest
from capsem_builder.cache.models import CachePolicy, PruneMethod, StagePolicy
from capsem_builder.cache.paths import CachePaths
from pydantic import ValidationError


def policy() -> CachePolicy:
    return CachePolicy(
        version=1,
        root=Path("cache"),
        minimum_free_bytes=1,
        stages={
            "cargo-debug": StagePolicy(
                path=Path("target/cargo/debug"),
                warning_bytes=10,
                soft_bytes=20,
                hard_bytes=30,
                prune=PruneMethod.LRU,
                maximum_age_hours=72,
            )
        },
    )


def test_paths_resolve_from_the_repository_root(tmp_path: Path) -> None:
    paths = CachePaths(repository_root=tmp_path, policy=policy())

    assert paths.root == tmp_path / "cache"
    assert paths.stage("cargo-debug") == tmp_path / "cache/target/cargo/debug"


def test_unknown_stage_fails_by_name(tmp_path: Path) -> None:
    paths = CachePaths(repository_root=tmp_path, policy=policy())

    with pytest.raises(KeyError, match="unknown cache stage 'missing'"):
        paths.stage("missing")


def test_repository_root_must_be_absolute(tmp_path: Path) -> None:
    with pytest.raises(ValidationError, match="repository root must be absolute"):
        CachePaths(repository_root=Path("relative"), policy=policy())


def test_configured_paths_must_live_inside_the_cache_root(tmp_path: Path) -> None:
    paths = CachePaths(repository_root=tmp_path, policy=policy())

    assert paths.resolve(Path("cache/target/assets")) == tmp_path / "cache/target/assets"
    with pytest.raises(ValueError, match="cache root"):
        paths.resolve(Path("target/assets"))


def test_path_resolver_is_strict_and_frozen(tmp_path: Path) -> None:
    paths = CachePaths(repository_root=tmp_path, policy=policy())

    with pytest.raises(ValidationError, match="Extra inputs"):
        CachePaths.model_validate(
            {"repository_root": tmp_path, "policy": policy(), "surprise": True}
        )
    with pytest.raises(ValidationError, match="frozen"):
        setattr(paths, "repository_" + "root", tmp_path / "other")
