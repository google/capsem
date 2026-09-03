"""Contracts for the unified cache policy."""

from pathlib import Path

import pytest
from capsem_builder.cache.config import load_policy
from capsem_builder.cache.models import CachePolicy, CacheScope, PruneStrategy, StagePolicy
from capsem_builder.cache.runtimemodels import DockerRuntimePolicy, TartRuntimePolicy
from pydantic import ValidationError

PROJECT_ROOT = Path(__file__).resolve().parents[3]


def stage(**overrides: object) -> StagePolicy:
    values = {
        "description": "Cargo test output",
        "scope": CacheScope.DISK,
        "path": "target/cargo/debug",
        "warm_size_bytes": 20,
        "max_size_bytes": 30,
        "prune_strategy": PruneStrategy.LRU,
        "maximum_age_hours": 72,
    }
    values.update(overrides)
    return StagePolicy.model_validate(values)


def test_stage_policy_is_strict_frozen_and_described() -> None:
    policy = stage()

    with pytest.raises(ValidationError, match="Extra inputs are not permitted"):
        StagePolicy.model_validate({**policy.model_dump(), "mystery": True})
    with pytest.raises(ValidationError, match="frozen"):
        setattr(policy, "max_" + "size_bytes", 40)
    with pytest.raises(ValidationError, match="visible text"):
        stage(description="  ")


def test_warm_size_cannot_exceed_maximum() -> None:
    with pytest.raises(ValidationError, match="warm_size_bytes cannot exceed max_size_bytes"):
        stage(warm_size_bytes=31, max_size_bytes=30)


@pytest.mark.parametrize("path", ["/tmp/cache", "../target", "cache/target/../escape", "."])
def test_stage_paths_are_relative_and_contained(path: str) -> None:
    with pytest.raises(ValidationError, match="relative descendant"):
        stage(path=path)


def test_stage_paths_must_be_unique_non_overlapping_leaves() -> None:
    with pytest.raises(ValidationError, match="overlap"):
        CachePolicy.model_validate(
            {
                "version": 1,
                "root": "cache",
                "authority_environment": "CAPSEM_TEST_CACHE_AUTHORITY",
                "stages": {
                    "cargo": stage(path="target/cargo").model_dump(mode="json"),
                    "debug": stage(path="target/cargo/debug").model_dump(mode="json"),
                },
            }
        )


def test_checked_in_policy_accounts_for_every_mechanism() -> None:
    policy = load_policy(PROJECT_ROOT)

    assert policy.root == Path("cache")
    assert policy.authority_environment == "CAPSEM_CACHE_AUTHORITY"
    assert policy.stages["cargo"].path == Path("target/cargo")
    assert policy.stages["assets"].entry_root == Path("generations")
    assert policy.stages["python-pycache"].managed_globs == ("cpython-*",)
    assert policy.stages["python-pycache"].lease_template == ".{key}.lock"
    assert isinstance(policy.runtimes["docker"], DockerRuntimePolicy)
    assert isinstance(policy.runtimes["tart"], TartRuntimePolicy)
    assert policy.control is not None
    assert policy.runtimes["docker"].warm_size_bytes == 72 * 1024**3
    assert policy.runtimes["docker"].max_size_bytes == 96 * 1024**3
    assert all(stage.description.strip() for stage in policy.stages.values())
    assert all(runtime.description.strip() for runtime in policy.runtimes.values())
    assert all(image.description.strip() for image in policy.control.docker.images.values())
    host_builder = policy.control.docker.images["capsem-host-builder"]
    assert host_builder.repository == "capsem-host-builder"
    assert host_builder.warm_size_bytes == 8 * 1024**3
    assert host_builder.max_size_bytes == 16 * 1024**3


def test_cache_authority_environment_is_required_and_canonical() -> None:
    values = {
        "version": 1,
        "root": "cache",
        "stages": {"cargo": stage().model_dump(mode="json")},
    }

    with pytest.raises(ValidationError, match="authority_environment"):
        CachePolicy.model_validate(values)
    with pytest.raises(ValidationError, match="authority_environment"):
        CachePolicy.model_validate({**values, "authority_environment": "not-valid"})


def test_bootstrap_reads_only_the_common_docker_contract() -> None:
    bootstrap = (PROJECT_ROOT / "bootstrap.sh").read_text(encoding="utf-8")

    assert 'capsem-cache --repository "$SCRIPT_DIR" contract docker' in bootstrap
    assert '["max_size_bytes"]' in bootstrap


def test_runtime_policy_references_an_owned_log_stage() -> None:
    docker = DockerRuntimePolicy(
        description="Docker cache",
        scope=CacheScope.DOCKER,
        warm_size_bytes=20,
        max_size_bytes=30,
        prune_strategy=PruneStrategy.DOCKER,
        kind="docker",
        command="docker",
        timeout_seconds=30,
        mutation_timeout_seconds=600,
        inventory_retry_attempts=3,
        inventory_retry_delay_milliseconds=0,
        log_stage="missing",
        image_prefixes=("capsem-",),
        container_prefixes=("capsem-",),
        volume_prefixes=("capsem-package-target-",),
        build_cache_owned=True,
        maximum_age_hours=72,
        keep_image_generations=2,
    )

    with pytest.raises(ValidationError, match="unknown stage 'missing'"):
        CachePolicy(
            version=1,
            root=Path("cache"),
            authority_environment="CAPSEM_TEST_CACHE_AUTHORITY",
            stages={"logs": stage(path="containers/logs")},
            runtimes={"docker": docker},
        )
