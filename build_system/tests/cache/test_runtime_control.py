"""Runtime retention is pure until one journaled adapter boundary applies it."""

from pathlib import Path

import pytest
from capsem_builder.cache.controlmodels import (
    CacheControlPolicy,
    CapacityRail,
    DockerControlPolicy,
    FailureArtifactPolicy,
    ImageCachePolicy,
    ReleaseBoundary,
)
from capsem_builder.cache.models import CachePolicy, PruneMethod, StagePolicy
from capsem_builder.cache.paths import CachePaths
from capsem_builder.cache.runtimeinventory import write_receipts
from capsem_builder.cache.runtimemodels import (
    DockerRuntimePolicy,
    ResourceKind,
    RuntimeCommandResult,
    RuntimeInventory,
    RuntimeKind,
    RuntimeOperation,
    RuntimeResource,
    RuntimeSnapshot,
)
from capsem_builder.cache.runtimeoperations import apply_runtime_prune
from capsem_builder.cache.runtimeplanner import (
    NANOSECONDS_PER_HOUR,
    plan_release,
    plan_repository_reclaim,
    plan_runtime_clean,
    plan_runtime_prune,
)

PINNED_PROBE = "debian:bookworm-slim@sha256:" + "a" * 64


def policy() -> CachePolicy:
    def stage(path: str) -> StagePolicy:
        return StagePolicy(
            path=Path(path),
            warning_bytes=1,
            soft_bytes=2,
            hard_bytes=3,
            prune=PruneMethod.LRU,
            maximum_age_hours=1,
        )

    return CachePolicy(
        version=1,
        root=Path("cache"),
        minimum_free_bytes=1,
        stages={"receipts": stage("containers/receipts"), "logs": stage("containers/logs")},
        runtimes={
            "docker": DockerRuntimePolicy(
                kind="docker",
                command="docker",
                timeout_seconds=30,
                mutation_timeout_seconds=600,
                receipt_stage="receipts",
                log_stage="logs",
                image_prefixes=("capsem-",),
                container_prefixes=("capsem-",),
                build_cache_owned=True,
                maximum_age_hours=72,
                keep_image_generations=1,
                build_cache_keep_bytes=80,
            )
        },
    )


def controlled_policy() -> CachePolicy:
    base = policy()
    control = CacheControlPolicy(
        docker=DockerControlPolicy(
            runtime_id="docker",
            capacity_probe_image=PINNED_PROBE,
            minimum_disk_bytes=100,
            recommended_disk_bytes=200,
            rails={"default": CapacityRail(minimum_free_bytes=10, build_cache_keep_bytes=80)},
            images={
                "tool": ImageCachePolicy(repository="capsem-tool", keep_previous=0),
            },
            releases={
                "after-tool": ReleaseBoundary(rail="default", images=("capsem-working:latest",))
            },
        ),
        failure_artifacts=FailureArtifactPolicy(
            stage="logs",
            minimum_count=1,
            maximum_count=2,
            maximum_age_hours=24,
            maximum_bytes=100,
            maximum_file_bytes=10,
            skip_names=(),
            source_patterns=("target/build.log",),
        ),
    )
    return CachePolicy.model_validate({**base.model_dump(), "control": control.model_dump()})


def test_docker_capacity_probe_requires_an_immutable_digest() -> None:
    values = controlled_policy().control
    assert values is not None
    with pytest.raises(ValueError, match="pinned by SHA-256 digest"):
        DockerControlPolicy.model_validate(
            {**values.docker.model_dump(), "capacity_probe_image": "debian:bookworm-slim"}
        )


def resource(
    kind: ResourceKind,
    identity: str,
    created: int,
    *,
    protected: bool = False,
    size: int = 10,
) -> RuntimeResource:
    return RuntimeResource(
        kind=kind,
        identity=identity,
        names=(("capsem-tool:" + identity) if kind is ResourceKind.IMAGE else identity,),
        logical_bytes=size,
        created_ns=created,
        last_used_ns=created,
        active=protected,
        owned=True,
        protected=protected,
    )


def test_runtime_plan_keeps_current_and_active_resources() -> None:
    now = 100 * NANOSECONDS_PER_HOUR
    inventory = RuntimeInventory(
        runtime_id="docker",
        kind=RuntimeKind.DOCKER,
        available=True,
        generated_ns=now,
        native_bytes=200,
        owned_bytes=200,
        resources=(
            resource(ResourceKind.IMAGE, "new", now - 1),
            resource(ResourceKind.IMAGE, "old", now - 2),
            resource(ResourceKind.CONTAINER, "stopped", now - 73 * NANOSECONDS_PER_HOUR),
            resource(ResourceKind.CONTAINER, "running", 1, protected=True),
            resource(ResourceKind.BUILD_CACHE, "buildkit", 0, size=100),
        ),
    )
    snapshot = RuntimeSnapshot(
        generated_ns=now, native_bytes=200, owned_bytes=200, runtimes=(inventory,)
    )

    plan = plan_runtime_prune(snapshot, policy())

    assert {(action.operation, action.target) for action in plan.actions} == {
        (RuntimeOperation.REMOVE_IMAGE, "capsem-tool:old"),
        (RuntimeOperation.REMOVE_CONTAINER, "stopped"),
        (RuntimeOperation.PRUNE_BUILD_CACHE, "buildkit"),
    }


def test_runtime_apply_uses_exact_argv_and_journals(tmp_path: Path) -> None:
    now = 1
    inventory = RuntimeInventory(
        runtime_id="docker",
        kind=RuntimeKind.DOCKER,
        available=True,
        generated_ns=now,
        native_bytes=100,
        owned_bytes=100,
        resources=(resource(ResourceKind.BUILD_CACHE, "buildkit", 0, size=100),),
    )
    plan = plan_runtime_prune(
        RuntimeSnapshot(generated_ns=now, native_bytes=100, owned_bytes=100, runtimes=(inventory,)),
        policy(),
    )
    issued = []

    def runner(argv: tuple[str, ...], _timeout: int) -> RuntimeCommandResult:
        issued.append(argv)
        return RuntimeCommandResult(
            argv=argv, returncode=0, stdout="reclaimed", stderr="", duration_ms=1
        )

    result = apply_runtime_prune(
        CachePaths(repository_root=tmp_path, policy=policy()),
        policy(),
        plan,
        reason="test policy",
        runner=runner,
    )

    assert issued == [
        (
            "docker",
            "builder",
            "prune",
            "--force",
            "--filter",
            "until=72h",
            "--keep-storage",
            "80B",
        )
    ]
    assert result.journal == tmp_path / "cache/containers/logs/runtime-mutations.jsonl"
    assert result.journal.is_file()


def test_runtime_snapshot_receipt_roundtrips_strictly(tmp_path: Path) -> None:
    inventory = RuntimeInventory(
        runtime_id="docker",
        kind=RuntimeKind.DOCKER,
        available=True,
        generated_ns=1,
        native_bytes=10,
        owned_bytes=10,
    )
    snapshot = RuntimeSnapshot(
        generated_ns=1, native_bytes=10, owned_bytes=10, runtimes=(inventory,)
    )
    paths = CachePaths(repository_root=tmp_path, policy=policy())

    (receipt,) = write_receipts(paths, policy(), snapshot)

    assert RuntimeInventory.model_validate_json(receipt.read_text(encoding="utf-8")) == inventory
    assert receipt == tmp_path / "cache/containers/receipts/docker.inventory.json"


def test_repository_reclaim_requires_present_anchor_and_preserves_receipts() -> None:
    now = 10
    inventory = RuntimeInventory(
        runtime_id="docker",
        kind=RuntimeKind.DOCKER,
        available=True,
        generated_ns=now,
        native_bytes=30,
        owned_bytes=30,
        resources=(
            resource(ResourceKind.IMAGE, "current", 3),
            resource(ResourceKind.IMAGE, "receipt", 2),
            resource(ResourceKind.IMAGE, "old", 1),
        ),
    )
    snapshot = RuntimeSnapshot(
        generated_ns=now, native_bytes=30, owned_bytes=30, runtimes=(inventory,)
    )

    plan = plan_repository_reclaim(
        snapshot,
        controlled_policy(),
        "tool",
        keep="capsem-tool:current",
        protect=("capsem-tool:receipt",),
    )

    assert tuple(action.target for action in plan.actions) == ("capsem-tool:old",)
    with pytest.raises(ValueError, match="absent; refusing unanchored reclaim"):
        plan_repository_reclaim(
            snapshot,
            controlled_policy(),
            "tool",
            keep="capsem-tool:missing",
        )


def test_release_removes_only_exact_inactive_working_image() -> None:
    working = RuntimeResource(
        kind=ResourceKind.IMAGE,
        identity="sha256:working",
        names=("capsem-working:latest",),
        logical_bytes=20,
        created_ns=1,
        last_used_ns=1,
        active=False,
        owned=True,
        protected=False,
    )
    inventory = RuntimeInventory(
        runtime_id="docker",
        kind=RuntimeKind.DOCKER,
        available=True,
        generated_ns=2,
        native_bytes=20,
        owned_bytes=20,
        resources=(working,),
    )
    snapshot = RuntimeSnapshot(
        generated_ns=2, native_bytes=20, owned_bytes=20, runtimes=(inventory,)
    )

    plan = plan_release(snapshot, controlled_policy(), "after-tool")

    assert [(item.operation, item.target) for item in plan.actions] == [
        (RuntimeOperation.REMOVE_IMAGE, "capsem-working:latest")
    ]


def test_cold_clean_never_selects_active_or_foreign_resources() -> None:
    inventory = RuntimeInventory(
        runtime_id="docker",
        kind=RuntimeKind.DOCKER,
        available=True,
        generated_ns=2,
        native_bytes=30,
        owned_bytes=20,
        resources=(
            resource(ResourceKind.CONTAINER, "stopped", 1),
            resource(ResourceKind.CONTAINER, "running", 1, protected=True),
            resource(ResourceKind.IMAGE, "foreign", 1).model_copy(update={"owned": False}),
        ),
    )
    snapshot = RuntimeSnapshot(
        generated_ns=2, native_bytes=30, owned_bytes=20, runtimes=(inventory,)
    )

    plan = plan_runtime_clean(snapshot, controlled_policy())

    assert [(item.operation, item.target) for item in plan.actions] == [
        (RuntimeOperation.REMOVE_CONTAINER, "stopped")
    ]
