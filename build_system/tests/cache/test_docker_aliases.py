"""A retained registry origin is another name for one owned image, not more bytes."""

import json
from pathlib import Path

import pytest
from capsem_builder.cache import dockeradapter
from capsem_builder.cache.config import load_policy
from capsem_builder.cache.dockerimages import plan_image_clean, plan_repository_reclaim
from capsem_builder.cache.runtimemodels import DockerRuntimePolicy
from capsem_builder.cache.runtimeplanner import plan_runtime_clean, plan_runtime_prune

from .test_docker_current import image, removed, snapshot
from .test_runtime_adapters import command
from .test_runtime_control import controlled_policy


@pytest.mark.parametrize("owner", ["clean", "image_clean", "reclaim", "prune"])
def test_image_retirement_removes_every_owned_alias_and_counts_bytes_once(owner):
    old = image("capsem-tool:old", 1, size=20).model_copy(
        update={"names": ("capsem-tool:old", "ghcr.io/google/capsem-ci/tool:old")},
    )
    current = image("capsem-tool:current", 2, size=20).model_copy(update={"protected": True})
    state = snapshot(old, current)
    policy = controlled_policy()
    if owner == "clean":
        plan = plan_runtime_clean(state, policy)
    elif owner == "image_clean":
        plan = plan_image_clean(state, policy, "tool")
    elif owner == "reclaim":
        plan = plan_repository_reclaim(state, policy, "tool", keep="capsem-tool:current")
    else:
        plan = plan_runtime_prune(state, policy)
    assert removed(plan) == set(old.names)
    assert plan.reclaim_bytes == old.logical_bytes
    assert sum(action.logical_bytes for action in plan.actions) == old.logical_bytes


def test_default_repository_retirement_removes_every_owned_alias():
    old = image("capsem-other:old", 1).model_copy(
        update={"names": ("capsem-other:old", "ghcr.io/google/capsem-ci/other:old")},
    )
    plan = plan_runtime_prune(snapshot(old, image("capsem-other:new", 2)), controlled_policy())
    assert removed(plan) == set(old.names)
    assert plan.reclaim_bytes == old.logical_bytes


def test_inventory_keeps_only_owned_origins_and_counts_one_image():
    policy = load_policy(Path(__file__).resolve().parents[3]).runtimes["docker"]
    assert isinstance(policy, DockerRuntimePolicy)
    assert "ghcr.io/google/capsem-ci/" in policy.image_prefixes
    names = ["capsem-tool:old", "ghcr.io/google/capsem-ci/tool:old", "ghcr.io/google/unrelated:one"]

    def runner(argv, _timeout):
        if argv[1:3] == ("image", "ls"):
            return command(argv, '\n'.join(json.dumps({"ID": "sha256:one", "Repository": name.rsplit(":", 1)[0]}) for name in names))
        assert argv[1:3] == ("image", "inspect")
        assert argv[-1:] == ("sha256:one",)
        return command(argv, 'sha256:one\\t2026-08-02T00:00:00Z\\t20\\t' + json.dumps(names))

    resources = dockeradapter._owned_images(policy, runner, set())
    assert len(resources) == 1
    assert resources[0].names == tuple(names[:2])
    plan = plan_runtime_clean(snapshot(*resources), controlled_policy())
    assert removed(plan) == set(names[:2])
    assert plan.reclaim_bytes == 20
