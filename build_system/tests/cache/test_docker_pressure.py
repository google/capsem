"""Per-repository warm generations cannot make the total Docker cap optional."""

import pytest
from capsem_builder.cache.dockerimages import plan_image_clean, plan_repository_reclaim
from capsem_builder.cache.runtimeplanner import plan_runtime_clean, plan_runtime_prune

from .test_docker_current import image, removed, snapshot
from .test_runtime_control import controlled_policy


@pytest.mark.parametrize("over_limit", [False, True])
def test_total_pressure_retires_least_used_images_without_current_or_live_ones(over_limit):
    oldest = image("capsem-cold-one:latest", 1, size=30 if over_limit else 5).model_copy(
        update={"names": ("capsem-cold-one:latest", "ghcr.io/google/capsem-ci/cold-one:latest")},
    )
    recent = image("capsem-cold-two:latest", 2, size=20 if over_limit else 5)
    current = image("capsem-current:latest", 3, size=30).model_copy(update={"current": True})
    protected = image("capsem-protected:latest", 4, size=20).model_copy(update={"protected": True})
    active = image("capsem-active:latest", 5, size=20).model_copy(update={"active": True})
    foreign = image("unrelated:latest", 0, size=200).model_copy(update={"owned": False})
    state = snapshot(oldest, recent, current, protected, active, foreign)
    inventory = state.runtimes[0].model_copy(update={"owned_bytes": 120 if over_limit else 80})
    state = state.model_copy(update={"runtimes": (inventory,), "owned_bytes": inventory.owned_bytes})
    plan = plan_runtime_prune(state, controlled_policy())
    assert removed(plan) == ({*oldest.names, *recent.names} if over_limit else set())
    assert plan.reclaim_bytes == (50 if over_limit else 0)
    assert plan.violations == ()
    assert all(action.logical_bytes >= 0 for action in plan.actions)


def test_unavoidable_pressure_reports_the_remaining_protected_bytes():
    current = image("capsem-current:latest", 1, size=100).model_copy(update={"current": True})
    plan = plan_runtime_prune(snapshot(current), controlled_policy())
    assert removed(plan) == set()
    assert plan.reclaim_bytes == 0
    assert any("100 owned bytes above max size 90" in error for error in plan.violations)


def test_global_pressure_does_not_leave_a_retired_child_capacity_violation():
    large = image("capsem-tool:latest", 1, size=100)
    plan = plan_runtime_prune(snapshot(large), controlled_policy())
    assert removed(plan) == set(large.names)
    assert plan.reclaim_bytes == 100
    assert plan.violations == ()


@pytest.mark.parametrize("owner", ["prune", "clean", "image_clean", "reclaim"])
def test_active_image_is_never_retired_even_without_a_redundant_protected_flag(owner):
    active = image("capsem-tool:active", 1).model_copy(update={"active": True})
    current = image("capsem-tool:current", 2).model_copy(update={"protected": True})
    state = snapshot(active, current)
    policy = controlled_policy()
    if owner == "prune":
        plan = plan_runtime_prune(state, policy)
    elif owner == "clean":
        plan = plan_runtime_clean(state, policy)
    elif owner == "image_clean":
        plan = plan_image_clean(state, policy, "tool")
    else:
        plan = plan_repository_reclaim(state, policy, "tool", keep="capsem-tool:current")
    assert removed(plan) == set()
    assert plan.reclaim_bytes == 0
