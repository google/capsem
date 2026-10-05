"""A phase can hold something against outsiders while its lanes share it.

The asset build runs two architecture lanes that must overlap to fit the time
budget, and must exclude every *other* Docker step while they do. The plan
could express neither half: an exclusive was a `threading.Lock`, so declaring
`docker_daemon` serialized the lanes, and not declaring it let any Docker step
schedule beside them.

`assetlanes.py` answered that with its own `ThreadPoolExecutor` -- concurrency
the graph cannot see, cannot order against, and cannot attribute a failure to.

So `contends` gains a mode. Shared holders admit each other and exclude
writers; an exclusive holder excludes everyone. Which is a readers-writer
lock, and is exactly the shape the problem had all along.
"""

from __future__ import annotations

import threading
import time
from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate.actions import Call
from capsem_builder.gate.contention import can_overlap
from capsem_builder.gate.context import Context
from capsem_builder.gate.execution import step
from capsem_builder.gate.module_contracts import ReleaseContractsModule
from capsem_builder.gate.opacity import CallJustification, OpaqueKind
from capsem_builder.gate.plan import Plan
from helpers.gate import RecordingRunner, gate_plan

PROJECT_ROOT = Path(__file__).resolve().parents[3]
CONFIG = gate_config.load(PROJECT_ROOT)

CONTRACT_CARGO_RATIONALE = (
    "Both contract collectors contain native Cargo fixtures. Inside an exclusive "
    "candidate the bounded wrapper correctly reuses the outer kernel lease; "
    "each collector must therefore claim workspace_binaries exclusively in the "
    "same plan, alongside its existing Astro/Node claims. See config/gate.toml "
    "[execution.exclusives.workspace_binaries] and skills/dev-gate/SKILL.md."
)


def _overlap_probe():
    """Records the greatest number of steps running at once."""
    state = {"now": 0, "peak": 0}
    guard = threading.Lock()

    def action(_context) -> None:
        with guard:
            state["now"] += 1
            state["peak"] = max(state["peak"], state["now"])
        time.sleep(0.05)
        with guard:
            state["now"] -= 1

    return state, action


def _run(plan: Plan) -> None:
    plan.run(Context(RecordingRunner(PROJECT_ROOT), CONFIG))


def test_shared_holders_run_together() -> None:
    state, action = _overlap_probe()
    plan = Plan("shared")
    shared = (CONFIG.shared("docker_daemon"),)
    for name in ("lane-a", "lane-b"):
        plan.add(
            step(
                name,
                Call(
                    name,
                    action,
                    justification=CallJustification(
                        kind=OpaqueKind.RUNTIME_DERIVED,
                        reason="a synthetic step whose work is decided by the test",
                        effects=frozenset(),
                    ),
                ),
                contends=shared,
            )
        )

    _run(plan)

    assert state["peak"] == 2, "declared lanes must be able to overlap"


def test_an_exclusive_holder_excludes_the_shared_ones() -> None:
    """The other half: while a writer holds it, no lane may start."""
    state, action = _overlap_probe()
    plan = Plan("mixed")
    plan.add(
        step(
            "lane-a",
            Call(
                "a",
                action,
                justification=CallJustification(
                    kind=OpaqueKind.RUNTIME_DERIVED,
                    reason="a synthetic step whose work is decided by the test",
                    effects=frozenset(),
                ),
            ),
            contends=(CONFIG.shared("docker_daemon"),),
        )
    )
    plan.add(
        step(
            "lane-b",
            Call(
                "b",
                action,
                justification=CallJustification(
                    kind=OpaqueKind.RUNTIME_DERIVED,
                    reason="a synthetic step whose work is decided by the test",
                    effects=frozenset(),
                ),
            ),
            contends=(CONFIG.shared("docker_daemon"),),
        )
    )
    plan.add(
        step(
            "outsider",
            Call(
                "o",
                action,
                justification=CallJustification(
                    kind=OpaqueKind.RUNTIME_DERIVED,
                    reason="a synthetic step whose work is decided by the test",
                    effects=frozenset(),
                ),
            ),
            contends=(CONFIG.exclusive("docker_daemon"),),
        )
    )

    _run(plan)

    assert state["peak"] == 2, (
        "an exclusive holder ran beside the shared ones; it must exclude them"
    )


def test_exclusive_holders_still_exclude_each_other() -> None:
    state, action = _overlap_probe()
    plan = Plan("exclusive")
    for name in ("one", "two"):
        plan.add(
            step(
                name,
                Call(
                    name,
                    action,
                    justification=CallJustification(
                        kind=OpaqueKind.RUNTIME_DERIVED,
                        reason="a synthetic step whose work is decided by the test",
                        effects=frozenset(),
                    ),
                ),
                contends=(CONFIG.exclusive("docker_daemon"),),
            )
        )

    _run(plan)

    assert state["peak"] == 1


def test_a_shared_claim_names_a_declared_exclusive() -> None:
    """Same rule as `exclusive`: inventing one contends with nothing."""
    from capsem_builder.gate.errors import GateError

    with pytest.raises(GateError, match="unknown exclusive"):
        CONFIG.shared("not-a-real-resource")


@pytest.mark.parametrize("command", ["candidate", "test-release-contracts"])
@pytest.mark.parametrize("label", ["contracts.release", "contracts.build-system"])
def test_contract_collectors_claim_the_workspace_their_cargo_fixtures_use(
    command: str, label: str,
) -> None:
    plan = gate_plan(command)
    plan.validate(CONFIG)
    assert plan.step_named(label).contends == (
        CONFIG.exclusive("astro_build"),
        CONFIG.exclusive("node_modules"),
        CONFIG.exclusive("workspace_binaries"),
    ), CONTRACT_CARGO_RATIONALE


@pytest.mark.parametrize("label", ["contracts.release", "contracts.build-system"])
def test_contract_collectors_exclude_real_workspace_writers_and_readers(label: str) -> None:
    plan = gate_plan("candidate")
    collector = plan.step_named(label)
    users = [
        other for other in plan.steps
        if other.label != label
        and any(claim.name == "workspace_binaries" for claim in other.contends)
    ]
    assert users, "the actual candidate no longer declares any workspace users"
    assert not [other.label for other in users if can_overlap(collector, other)], (
        CONTRACT_CARGO_RATIONALE
    )


def test_standalone_contract_command_does_not_take_a_nonreentrant_outer_lease() -> None:
    assert not ReleaseContractsModule.exclusive, (
        "The standalone owner runs gate kernel-lease regressions; taking the "
        "outer lease would deadlock its child probes. Keep Cargo fixture leases "
        "in the bounded wrapper and same-plan contention on the actual steps."
    )
