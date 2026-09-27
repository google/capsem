"""Every gate command that compiles holds the shared Cargo target to its contract.

The contract is `[stages.cargo]` in `config/cache.toml`, and it was enforced as
a plan step that only the host-build fragment added, in front of its build.
Everything else compiled into the same shared target with nothing in front of
it -- clippy in `test-fast`, the coverage build in `test-static`,
`test-rust-affected`, `smoke`, `pack-initrd` -- and nothing enforced after any
compile at all, so the stage was left above its maximum by whatever the last
run added. Every worktree's checkout path salts a fresh set of workspace units,
so those unbounded paths accumulated until the target reached 237 GB against a
180 GiB maximum and a `focus-test` died with ENOSPC.

The bound is therefore a resource of the command, not a step one fragment
remembers: acquired before the first step and released after the last, on
every path including a failed one.
"""

from __future__ import annotations

from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate import preflight
from capsem_builder.gate.cachecontrol import CargoCacheBound
from capsem_builder.gate.lifecycle import held
from helpers.gate import RecordingRunner, gate_plan

ROOT = Path(__file__).resolve().parents[3]
CONFIG = gate_config.load(ROOT)
ENFORCE = r"capsem-cache .* enforce cargo --reason"

#: Exclusive commands whose plans compile into the shared target. The first
#: seven had no Cargo enforcement anywhere in their plans.
COMPILING = (
    "test-fast",
    "test-static",
    "test-rust-affected",
    "bench-report",
    "smoke",
    "pack-initrd",
    "check-assets",
    "test-functional",
    "build-host",
    "candidate",
)


def _holdings(command: str) -> tuple:
    plan = gate_plan(command)
    return preflight.holdings(
        CONFIG, RecordingRunner(ROOT), command, exclusive=True, declared=(), plan=plan
    )


@pytest.mark.parametrize("command", COMPILING)
def test_every_compiling_command_holds_the_cargo_bound(command: str) -> None:
    kinds = [type(resource) for resource in _holdings(command)]
    assert CargoCacheBound in kinds, (
        f"{command} compiles into the shared Cargo target without enforcing its maximum"
    )


def test_a_plan_that_never_compiles_is_not_charged_for_the_scan() -> None:
    assert not preflight.compiles(gate_plan("host-image"), CONFIG)
    assert CargoCacheBound not in [type(resource) for resource in _holdings("host-image")]


def test_a_command_without_the_machine_never_mutates_the_cache() -> None:
    held_resources = preflight.holdings(
        CONFIG,
        RecordingRunner(ROOT),
        "test-release-contracts",
        exclusive=False,
        declared=(),
        plan=gate_plan("test-release-contracts"),
    )
    assert CargoCacheBound not in [type(resource) for resource in held_resources]


def _acting() -> RecordingRunner:
    runner = RecordingRunner(ROOT)
    runner.observing = False
    return runner


def test_the_bound_enforces_before_the_first_step_and_after_the_last() -> None:
    runner = _acting()
    with held(CargoCacheBound(runner)):
        assert len(runner.matching(ENFORCE)) == 1, "nothing enforced before compilation"
    enforced = runner.matching(ENFORCE)
    assert len(enforced) == 2, "nothing enforced after compilation"
    assert "before" in enforced[0] and "after" in enforced[1]


def test_a_failed_plan_still_leaves_the_target_inside_its_contract() -> None:
    runner = _acting()
    with pytest.raises(RuntimeError), held(CargoCacheBound(runner)):
        raise RuntimeError("the compile ran out of disk")
    assert len(runner.matching(ENFORCE)) == 2


def test_asking_what_a_plan_would_do_prunes_nothing() -> None:
    runner = RecordingRunner(ROOT)
    assert runner.observing
    with held(CargoCacheBound(runner)):
        pass
    assert not runner.commands
