"""Fresh Docker tags remain safe reclaim anchors during inventory convergence."""

from pathlib import Path

from capsem_builder.cache import controlcli
from capsem_builder.cache.dockeradapter import inspect_image
from capsem_builder.cache.dockerimages import plan_repository_reclaim
from capsem_builder.cache.paths import CachePaths
from capsem_builder.cache.runtimemodels import (
    DockerRuntimePolicy,
    ResourceKind,
    RuntimeCommandResult,
    RuntimeInventory,
    RuntimeKind,
    RuntimeResource,
    RuntimeSnapshot,
)
from click.testing import CliRunner
from pytest import MonkeyPatch

from .test_runtime_control import controlled_policy, resource


def test_exact_inspection_returns_a_protected_typed_anchor() -> None:
    policy = controlled_policy().runtimes["docker"]
    assert isinstance(policy, DockerRuntimePolicy)

    def runner(argv: tuple[str, ...], _timeout: int) -> RuntimeCommandResult:
        assert argv[-1] == "capsem-tool:fresh"
        return RuntimeCommandResult(
            argv=argv,
            returncode=0,
            stdout=(
                'sha256:fresh\\t2026-09-04T03:34:03Z\\t12'
                '\\t["capsem-tool:fresh"]\n'
            ),
            stderr="",
            duration_ms=1,
        )

    anchor = inspect_image(policy, "capsem-tool:fresh", runner=runner)

    assert anchor is not None
    assert anchor.kind is ResourceKind.IMAGE
    assert anchor.names == ("capsem-tool:fresh",)
    assert anchor.protected and anchor.owned


def test_exact_anchor_allows_reclaim_while_bulk_inventory_converges() -> None:
    old = resource(ResourceKind.IMAGE, "old", 1)
    inventory = RuntimeInventory(
        runtime_id="docker",
        kind=RuntimeKind.DOCKER,
        available=True,
        generated_ns=2,
        native_bytes=10,
        owned_bytes=10,
        resources=(old,),
    )
    snapshot = RuntimeSnapshot(
        generated_ns=2,
        native_bytes=10,
        owned_bytes=10,
        runtimes=(inventory,),
    )
    anchor = RuntimeResource(
        kind=ResourceKind.IMAGE,
        identity="sha256:fresh",
        names=("capsem-tool:fresh",),
        logical_bytes=12,
        created_ns=2,
        last_used_ns=2,
        active=False,
        owned=True,
        protected=True,
    )

    plan = plan_repository_reclaim(
        snapshot,
        controlled_policy(),
        "tool",
        keep="capsem-tool:fresh",
        anchor=anchor,
    )

    assert tuple(action.target for action in plan.actions) == ("capsem-tool:old",)


def test_reclaim_command_verifies_a_fresh_anchor_exactly(
    tmp_path: Path, monkeypatch: MonkeyPatch
) -> None:
    configured = controlled_policy()
    old = resource(ResourceKind.IMAGE, "old", 1)
    inventory = RuntimeInventory(
        runtime_id="docker",
        kind=RuntimeKind.DOCKER,
        available=True,
        generated_ns=2,
        native_bytes=10,
        owned_bytes=10,
        resources=(old,),
    )
    snapshot = RuntimeSnapshot(
        generated_ns=2,
        native_bytes=10,
        owned_bytes=10,
        runtimes=(inventory,),
    )
    anchor = old.model_copy(
        update={
            "identity": "sha256:fresh",
            "names": ("capsem-tool:fresh",),
            "protected": True,
        }
    )
    paths = CachePaths(repository_root=tmp_path, policy=configured)
    monkeypatch.setattr(
        controlcli,
        "_state",
        lambda _context, **_kwargs: (configured, paths, snapshot),
    )
    monkeypatch.setattr(controlcli.dockeradapter, "inspect_image", lambda *_args, **_kwargs: anchor)

    result = CliRunner().invoke(
        controlcli.reclaim_image,
        ("tool", "--keep", "capsem-tool:fresh"),
        obj={"repository": tmp_path, "policy_repository": tmp_path},
    )

    assert result.exit_code == 0, result.output
    assert '"target": "capsem-tool:old"' in result.output
