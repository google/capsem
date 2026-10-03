"""Docker removals give the bytes back to the host disk.

On macOS Docker runs inside Colima's VM, whose disk image only shrinks when
the guest discards freed blocks. Pruning Docker to 18 GiB left the image at
196 GB until `fstrim` ran by hand, which returned 130 GiB. The runtime
policy names the command that releases the disk; it runs once after any
applied Docker removal, is journaled with the removals, and is skipped where
its executable does not exist (native Docker on Linux has no VM disk).
"""

from __future__ import annotations

from pathlib import Path

import pytest
from capsem_builder.cache import runtimeoperations
from capsem_builder.cache.models import CachePolicy, CacheScope, PruneStrategy, StagePolicy
from capsem_builder.cache.paths import CachePaths
from capsem_builder.cache.runtimemodels import (
    DockerRuntimePolicy,
    RuntimeCommandResult,
    RuntimeOperation,
    RuntimePruneAction,
    RuntimePrunePlan,
)

RELEASE = ("colima", "ssh", "--", "sudo", "fstrim", "-a")


def policy(release: tuple[str, ...]) -> CachePolicy:
    return CachePolicy(
        version=1,
        root=Path("cache"),
        authority_environment="CAPSEM_TEST_CACHE_AUTHORITY",
        stages={
            "logs": StagePolicy(
                path=Path("containers/logs"),
                description="test cache",
                scope=CacheScope.DISK,
                warm_size_bytes=2,
                max_size_bytes=3,
                prune_strategy=PruneStrategy.LRU,
                maximum_age_hours=1,
            )
        },
        runtimes={
            "docker": DockerRuntimePolicy(
                description="Docker test cache",
                scope=CacheScope.DOCKER,
                warm_size_bytes=80,
                max_size_bytes=90,
                prune_strategy=PruneStrategy.DOCKER,
                kind="docker",
                command="docker",
                timeout_seconds=1,
                mutation_timeout_seconds=1,
                inventory_retry_attempts=1,
                inventory_retry_delay_milliseconds=0,
                log_stage="logs",
                image_prefixes=("capsem-",),
                container_prefixes=("capsem-",),
                volume_prefixes=("capsem-",),
                build_cache_owned=True,
                maximum_age_hours=1,
                keep_image_generations=1,
                disk_release_command=release,
            )
        },
    )


def removal(target: str) -> RuntimePruneAction:
    return RuntimePruneAction(
        runtime_id="docker",
        operation=RuntimeOperation.REMOVE_IMAGE,
        target=target,
        logical_bytes=10,
        reason="test",
    )


def apply(tmp_path: Path, release: tuple[str, ...], actions, returncode: int = 0):
    issued: list[tuple[str, ...]] = []

    def runner(argv: tuple[str, ...], _timeout: int) -> RuntimeCommandResult:
        issued.append(argv)
        code = returncode if argv[:2] == ("docker", "image") else 0
        return RuntimeCommandResult(argv=argv, returncode=code, stdout="", stderr="", duration_ms=1)

    configured = policy(release)
    result = runtimeoperations.apply_runtime_prune(
        CachePaths(repository_root=tmp_path, policy=configured),
        configured,
        RuntimePrunePlan(generated_ns=1, reclaim_bytes=0, actions=tuple(actions), violations=()),
        reason="test",
        runner=runner,
    )
    return issued, result


def test_removals_release_the_disk_once_and_journal_it(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(runtimeoperations.shutil, "which", lambda name: f"/usr/bin/{name}")
    issued, result = apply(tmp_path, RELEASE, (removal("capsem-a"), removal("capsem-b")))

    assert issued[-1] == RELEASE and issued.count(RELEASE) == 1
    released = result.results[-1]
    assert released.action.operation is RuntimeOperation.RELEASE_DISK
    assert result.journal is not None and "release-disk" in result.journal.read_text()


def test_no_release_without_a_successful_removal(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(runtimeoperations.shutil, "which", lambda name: f"/usr/bin/{name}")
    issued, _ = apply(tmp_path, RELEASE, (removal("capsem-a"),), returncode=1)
    assert RELEASE not in issued


def test_release_is_skipped_where_its_executable_is_absent(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(runtimeoperations.shutil, "which", lambda name: None)
    issued, result = apply(tmp_path, RELEASE, (removal("capsem-a"),))
    assert RELEASE not in issued
    assert all(
        entry.action.operation is not RuntimeOperation.RELEASE_DISK for entry in result.results
    )


def test_a_policy_without_a_release_command_releases_nothing(tmp_path: Path) -> None:
    issued, _ = apply(tmp_path, (), (removal("capsem-a"),))
    assert issued == [("docker", "image", "rm", "capsem-a")]
