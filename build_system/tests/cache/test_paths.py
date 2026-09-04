"""Path resolution must keep every persistent byte inside repository cache/."""

from pathlib import Path

import pytest
from capsem_builder.cache.config import load_paths
from capsem_builder.cache.models import CachePolicy, CacheScope, PruneStrategy, StagePolicy
from capsem_builder.cache.paths import CachePaths
from pydantic import ValidationError


def policy() -> CachePolicy:
    return CachePolicy(
        version=1,
        root=Path("cache"),
        authority_environment="CAPSEM_TEST_CACHE_AUTHORITY",
        stages={
            "cargo-debug": StagePolicy(
                path=Path("target/cargo/debug"),
                description="test cache",
                scope=CacheScope.DISK,
                warm_size_bytes=20,
                max_size_bytes=30,
                prune_strategy=PruneStrategy.LRU,
                maximum_age_hours=72,
            )
        },
    )


def _write_policy(root: Path) -> None:
    config = root / "config"
    config.mkdir(parents=True)
    config.joinpath("cache.toml").write_text(
        """
version = 1
root = "cache"
authority_environment = "CAPSEM_TEST_CACHE_AUTHORITY"
[stages.objects]
description = "immutable test objects"
scope = "disk"
path = "target/objects"
warm_size_bytes = 2
max_size_bytes = 3
prune_strategy = "lru"
maximum_age_hours = 72
""".strip(),
        encoding="utf-8",
    )


def test_paths_resolve_from_the_repository_root(tmp_path: Path) -> None:
    paths = CachePaths(repository_root=tmp_path, policy=policy())

    assert paths.root == tmp_path / "cache"
    assert paths.stage("cargo-debug") == tmp_path / "cache/target/cargo/debug"


def test_external_stage_resolves_outside_the_repository(tmp_path: Path) -> None:
    external = tmp_path / "scratch" / "capsem-tests"
    external_policy = policy().model_copy(
        update={
            "stages": {
                "test-temp": StagePolicy(
                    path=external,
                    external=True,
                    description="test scratch",
                    scope=CacheScope.DISK,
                    warm_size_bytes=20,
                    max_size_bytes=30,
                    prune_strategy=PruneStrategy.EPHEMERAL,
                    maximum_age_hours=1,
                )
            }
        }
    )

    paths = CachePaths(repository_root=tmp_path / "repository", policy=external_policy)

    assert paths.stage("test-temp").parent == external
    assert len(paths.stage("test-temp").name) == 8


def test_external_stage_cannot_be_inside_the_repository(tmp_path: Path) -> None:
    repository = tmp_path / "repository"
    external_policy = policy().model_copy(
        update={
            "stages": {
                "test-temp": StagePolicy(
                    path=repository / "scratch" / "capsem-tests",
                    external=True,
                    description="unsafe scratch",
                    scope=CacheScope.DISK,
                    warm_size_bytes=20,
                    max_size_bytes=30,
                    prune_strategy=PruneStrategy.EPHEMERAL,
                    maximum_age_hours=1,
                )
            }
        }
    )

    with pytest.raises(ValidationError, match="must be outside the repository"):
        CachePaths(repository_root=repository, policy=external_policy)


def test_linked_worktree_defaults_to_the_git_common_checkout(tmp_path: Path) -> None:
    common = tmp_path / "common"
    linked = tmp_path / "linked"
    git_dir = common / ".git/worktrees/linked"
    git_dir.mkdir(parents=True)
    linked.mkdir()
    (git_dir / "commondir").write_text("../..\n", encoding="utf-8")
    (linked / ".git").write_text(f"gitdir: {git_dir}\n", encoding="utf-8")
    _write_policy(linked)

    paths = load_paths(linked, environment={})

    assert paths.repository_root == common


def test_explicit_authority_overrides_the_git_common_checkout(tmp_path: Path) -> None:
    repository = tmp_path / "repository"
    authority = tmp_path / "authority"
    repository.mkdir()
    (repository / ".git").mkdir()
    _write_policy(repository)

    paths = load_paths(
        repository,
        environment={"CAPSEM_TEST_CACHE_AUTHORITY": str(authority)},
    )

    assert paths.repository_root == authority


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
