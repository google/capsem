"""Retention never removes an image generation a live checkout declared current.

On 2026-09-29 routine enforcement removed the running release proof's current
`capsem-kernel-dependencies-x86_64` generation (newest-by-creation is not
current: a cache-hit rebuild keeps its old timestamp, and each profile has its
own tag), and an early enforce evicted the current host builder. Each rebuild
cost the proof its slowest network phase again.
"""

from __future__ import annotations

import json
import time
from pathlib import Path

import pytest
from capsem_builder.cache import controlcli, dockercurrent
from capsem_builder.cache.dockerimages import plan_repository_reclaim
from capsem_builder.cache.enforcement import enforce_runtime
from capsem_builder.cache.paths import CachePaths
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
from capsem_builder.cache.runtimeplanner import (
    NANOSECONDS_PER_HOUR,
    plan_runtime_clean,
    plan_runtime_prune,
)
from click.testing import CliRunner

from .test_runtime_control import controlled_policy

NOW = 1_000 * NANOSECONDS_PER_HOUR


def image(tag: str, created: int, *, size: int = 10) -> RuntimeResource:
    return RuntimeResource(
        kind=ResourceKind.IMAGE,
        identity="sha256:" + tag,
        names=(tag,),
        logical_bytes=size,
        created_ns=created,
        last_used_ns=created,
        active=False,
        owned=True,
        protected=False,
    )


def snapshot(*resources: RuntimeResource) -> RuntimeSnapshot:
    total = sum(item.logical_bytes for item in resources)
    inventory = RuntimeInventory(
        runtime_id="docker",
        kind=RuntimeKind.DOCKER,
        available=True,
        generated_ns=NOW,
        native_bytes=total,
        owned_bytes=total,
        resources=resources,
    )
    return RuntimeSnapshot(generated_ns=NOW, native_bytes=total, owned_bytes=total, runtimes=(inventory,))


@pytest.fixture
def paths(tmp_path: Path) -> CachePaths:
    return CachePaths(repository_root=tmp_path / "authority", policy=controlled_policy())


@pytest.fixture
def checkout(tmp_path: Path) -> Path:
    path = tmp_path / "checkout"
    path.mkdir()
    return path


def removed(plan) -> set[str]:
    return {item.target for item in plan.actions if item.operation is RuntimeOperation.REMOVE_IMAGE}


def test_an_older_current_generation_survives_a_newer_superseded_one(paths, checkout) -> None:
    # The kernel-dependencies flap: the release proof's tag was built earlier
    # than another checkout's, and "retain newest 1" removed the one in use.
    dockercurrent.record(paths, tag="capsem-deps:proof", checkout=checkout, now_ns=NOW)
    state = dockercurrent.mark(
        snapshot(image("capsem-deps:proof", 1), image("capsem-deps:other", 2)), paths
    )

    assert "capsem-deps:proof" not in removed(plan_runtime_prune(state, controlled_policy()))


def test_every_profile_slot_keeps_its_own_current_tag(paths, checkout) -> None:
    for profile in ("code", "co-work"):
        dockercurrent.record(
            paths, tag=f"capsem-deps:{profile}", checkout=checkout, slot=profile, now_ns=NOW
        )
    state = dockercurrent.mark(
        snapshot(image("capsem-deps:code", 1), image("capsem-deps:co-work", 2)), paths
    )

    assert removed(plan_runtime_prune(state, controlled_policy())) == set()


def test_pressure_expiry_and_count_never_select_a_current_tag(paths, checkout) -> None:
    # Declared repository `capsem-tool`: max 30, warm 20. Three 20-byte tags,
    # all far past any age: pressure, expiry and count would each take the
    # older two. The current one must stay and the shortfall is reported.
    dockercurrent.record(paths, tag="capsem-tool:current", checkout=checkout, now_ns=NOW)
    state = dockercurrent.mark(
        snapshot(
            image("capsem-tool:newest", 3, size=20),
            image("capsem-tool:current", 2, size=20),
            image("capsem-tool:old", 1, size=20),
        ),
        paths,
    )

    plan = plan_runtime_prune(state, controlled_policy())

    assert removed(plan) == {"capsem-tool:old"}
    assert any("capsem-tool remains" in violation for violation in plan.violations)


def test_reclaim_keeps_another_checkouts_current_and_retires_its_own_previous(
    paths, checkout, tmp_path
) -> None:
    other = tmp_path / "proof"
    other.mkdir()
    dockercurrent.record(paths, tag="capsem-tool:proof", checkout=other, now_ns=NOW)
    dockercurrent.record(paths, tag="capsem-tool:previous", checkout=checkout, now_ns=NOW - 1)
    dockercurrent.record(paths, tag="capsem-tool:mine", checkout=checkout, now_ns=NOW)
    state = dockercurrent.mark(
        snapshot(
            image("capsem-tool:mine", 3),
            image("capsem-tool:proof", 2),
            image("capsem-tool:previous", 1),
        ),
        paths,
    )

    plan = plan_repository_reclaim(state, controlled_policy(), "tool", keep="capsem-tool:mine")

    assert removed(plan) == {"capsem-tool:previous"}


def test_a_record_dies_with_its_checkout_or_its_age(paths, checkout, tmp_path) -> None:
    gone = tmp_path / "removed-worktree"
    dockercurrent.record(paths, tag="capsem-tool:gone", checkout=gone, now_ns=NOW)
    runtime = controlled_policy().runtimes["docker"]
    assert isinstance(runtime, DockerRuntimePolicy)
    age = runtime.maximum_age_hours * NANOSECONDS_PER_HOUR
    dockercurrent.record(paths, tag="capsem-tool:stale", checkout=checkout, slot="a", now_ns=NOW - age - 1)
    dockercurrent.record(paths, tag="capsem-tool:live", checkout=checkout, slot="b", now_ns=NOW - age)

    assert dockercurrent.live_tags(paths, now_ns=NOW) == {"capsem-tool:live"}


def test_a_malformed_record_is_no_record(paths, checkout) -> None:
    written = dockercurrent.record(paths, tag="capsem-tool:live", checkout=checkout, now_ns=NOW)
    (written.parent / "broken.json").write_text("{", encoding="utf-8")
    (written.parent / "foreign.json").write_text(json.dumps({"tag": "capsem-tool:x"}), encoding="utf-8")

    assert dockercurrent.live_tags(paths, now_ns=NOW) == {"capsem-tool:live"}


def test_a_record_names_an_exact_tag_from_an_absolute_checkout(paths, checkout) -> None:
    with pytest.raises(ValueError, match="exact tag"):
        dockercurrent.record(paths, tag="capsem-tool", checkout=checkout)
    with pytest.raises(ValueError, match="absolute"):
        dockercurrent.record(paths, tag="capsem-tool:x", checkout=Path("relative"))


def test_explicit_cold_clean_may_still_remove_current_images(paths, checkout) -> None:
    dockercurrent.record(paths, tag="capsem-tool:current", checkout=checkout, now_ns=NOW)
    state = dockercurrent.mark(snapshot(image("capsem-tool:current", 1)), paths)

    assert removed(plan_runtime_clean(state, controlled_policy())) == {"capsem-tool:current"}


def test_enforcement_consults_the_records_it_is_given(paths, checkout) -> None:
    """End to end through the adapter: enforce over the maximum keeps the current tag."""
    dockercurrent.record(paths, tag="capsem-tool:current", checkout=checkout)
    images = {"capsem-tool:current": 1, "capsem-tool:newer": 2, "capsem-tool:newest": 3}
    issued: list[tuple[str, ...]] = []

    def runner(argv: tuple[str, ...], _timeout: int) -> RuntimeCommandResult:
        issued.append(argv)
        stdout = ""
        if argv[1:3] == ("system", "df"):
            stdout = json.dumps(
                {"Type": "Images", "TotalCount": "3", "Active": "0", "Size": "60B", "Reclaimable": "0B"}
            )
        elif argv[1:3] == ("image", "ls"):
            stdout = "\n".join(
                json.dumps({"ID": "sha256:" + tag, "Repository": tag.split(":")[0]}) for tag in images
            )
        elif argv[1:3] == ("image", "inspect"):
            stdout = "\n".join(
                f"sha256:{tag}\\t1970-01-01T00:00:0{created}Z\\t20\\t{json.dumps([tag])}"
                for tag, created in images.items()
            )
        return RuntimeCommandResult(argv=argv, returncode=0, stdout=stdout, stderr="", duration_ms=1)

    enforce_runtime(paths, controlled_policy(), "docker", reason="test", runner=runner)

    removals = {argv[-1] for argv in issued if argv[1:3] == ("image", "rm")}
    assert "capsem-tool:current" not in removals
    assert removals  # the superseded generations still go


def test_reclaim_command_records_the_checkouts_current_tag_before_planning(
    tmp_path: Path, checkout: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    authority = tmp_path / "authority"
    paths = CachePaths(repository_root=authority, policy=controlled_policy())
    seen: list[frozenset[str]] = []

    def state(_context, **_arguments):
        seen.append(dockercurrent.live_tags(paths, now_ns=time.time_ns()))
        return controlled_policy(), paths, snapshot()

    monkeypatch.setattr(controlcli, "load_policy", lambda _root: controlled_policy())
    monkeypatch.setattr(controlcli, "_state", state)

    result = CliRunner().invoke(
        controlcli.reclaim_image,
        ("tool", "--keep", "capsem-tool:mine", "--apply"),
        obj={"repository": authority, "policy_repository": checkout},
    )

    assert result.exit_code != 0  # the anchor is absent from the empty inventory
    assert seen == [frozenset({"capsem-tool:mine"})]
