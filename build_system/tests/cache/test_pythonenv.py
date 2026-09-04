"""Python cache selection is exact, strict, and canonical."""

from pathlib import Path

import pytest
from capsem_builder.cache.models import CachePolicy, CacheScope, PruneStrategy, StagePolicy
from capsem_builder.cache.paths import CachePaths
from capsem_builder.cache.pythonenv import PythonCacheEnvironment, select
from pydantic import ValidationError


def _paths(root: Path) -> CachePaths:
    def stage(path: str) -> StagePolicy:
        return StagePolicy(
            path=Path(path),
            description="test cache",
            scope=CacheScope.DISK,
            warm_size_bytes=2,
            max_size_bytes=3,
            prune_strategy=PruneStrategy.LRU,
            maximum_age_hours=1,
        )

    policy = CachePolicy(
        version=1,
        root=Path("cache"),
        authority_environment="CAPSEM_TEST_CACHE_AUTHORITY",
        stages={
            "python-pycache": stage("tools/python/pycache"),
            "python-pytest": stage("tools/python/pytest"),
        },
    )
    return CachePaths(repository_root=root, policy=policy)


def test_selection_replaces_every_inherited_pytest_cache_override(tmp_path: Path) -> None:
    paths = _paths(tmp_path)
    generation = paths.stage("python-pycache") / "cpython-312-source"

    selected = select(
        paths,
        generation,
        inherited_addopts=(
            "-q -o cache_dir=/old --override-ini=cache_dir=/older "
            "-o=cache_dir=/oldest --override-ini=cache_dir=/ancient -x"
        ),
    )

    assert selected.pytest_addopts.count("cache_dir=") == 1
    assert "/old" not in selected.pytest_addopts
    assert selected.pytest_addopts.startswith("-q -x ")
    assert selected.pytest_cache == paths.stage("python-pytest") / generation.name


def test_selection_rejects_a_pycache_outside_its_policy_stage(tmp_path: Path) -> None:
    with pytest.raises(ValueError, match="outside its policy stage"):
        select(_paths(tmp_path), tmp_path / "elsewhere/cpython-312-source")


def test_environment_schema_is_strict_and_frozen(tmp_path: Path) -> None:
    paths = _paths(tmp_path)
    selected = select(paths, paths.stage("python-pycache") / "cpython-312-source")

    with pytest.raises(ValidationError, match="Input should be an instance of Path"):
        PythonCacheEnvironment.model_validate(
            {**selected.model_dump(), "pytest_cache": str(selected.pytest_cache)}
        )
    field = "pytest_addopts"
    with pytest.raises(ValidationError, match="frozen"):
        setattr(selected, field, "-q")
