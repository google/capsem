"""Contracts for the repository-owned cache policy."""

from pathlib import Path

import pytest
from capsem_builder.cache.config import load_policy
from capsem_builder.cache.models import CachePolicy, PruneMethod, StagePolicy
from pydantic import ValidationError

PROJECT_ROOT = Path(__file__).resolve().parents[3]


def stage(**overrides: object) -> StagePolicy:
    values = {
        "path": "target/cargo/debug",
        "warning_bytes": 10,
        "soft_bytes": 20,
        "hard_bytes": 30,
        "prune": PruneMethod.LRU,
        "maximum_age_hours": 72,
    }
    values.update(overrides)
    return StagePolicy.model_validate(values)


def test_stage_policy_is_strict_and_frozen() -> None:
    policy = stage()

    with pytest.raises(ValidationError, match="Extra inputs are not permitted"):
        StagePolicy.model_validate({**policy.model_dump(), "mystery": True})
    with pytest.raises(ValidationError, match="frozen"):
        policy.soft_bytes = 40


@pytest.mark.parametrize(
    ("warning", "soft", "hard"),
    [(20, 10, 30), (10, 30, 20)],
)
def test_stage_limits_must_be_ordered(warning: int, soft: int, hard: int) -> None:
    with pytest.raises(ValidationError, match="warning_bytes <= soft_bytes <= hard_bytes"):
        stage(warning_bytes=warning, soft_bytes=soft, hard_bytes=hard)


@pytest.mark.parametrize("path", ["/tmp/cache", "../target", "target/../escape", "."])
def test_stage_paths_are_relative_and_contained(path: str) -> None:
    with pytest.raises(ValidationError, match="relative descendant"):
        stage(path=path)


def test_stage_paths_must_be_unique_non_overlapping_leaves() -> None:
    with pytest.raises(ValidationError, match="overlap"):
        CachePolicy.model_validate(
            {
                "version": 1,
                "root": "cache",
                "minimum_free_bytes": 1,
                "stages": {
                    "cargo": stage(path="target/cargo").model_dump(mode="json"),
                    "debug": stage(path="target/cargo/debug").model_dump(mode="json"),
                },
            }
        )


def test_checked_in_policy_loads_and_names_stage_owned_directories() -> None:
    policy = load_policy(PROJECT_ROOT)

    assert policy.root == Path("cache")
    assert policy.minimum_free_bytes == 40 * 1024**3
    assert policy.stages["cargo-debug"].path == Path("target/cargo/debug")
    assert policy.stages["python-pycache"].path == Path("tools/python/pycache")
    assert policy.stages["buildkit"].external is True
