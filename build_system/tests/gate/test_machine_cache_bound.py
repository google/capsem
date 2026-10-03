"""Every gate command that holds the machine leaves every cache inside its policy.

Only the Cargo target was held to its contract around each command; the
other stages were pruned by a few rails or by nothing. On 2026-10-03 this
machine carried 248 GB of dead test runs in `test-temp` (max 32 GiB), 78 GB
of superseded Docker images (max 40 GiB) and 34 GB of unreferenced objects
(max 12 GiB) -- all of it reclaimable by policy, none of it ever reclaimed.

Routine retention is therefore a resource of every exclusive command,
released after the last step and after everything else it holds, on every
path including a failed one.
"""

from __future__ import annotations

from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate import preflight
from capsem_builder.gate.cachecontrol import MachineCacheBound
from capsem_builder.gate.lifecycle import held
from helpers.gate import RecordingRunner, gate_plan

ROOT = Path(__file__).resolve().parents[3]
CONFIG = gate_config.load(ROOT)
PRUNE = r"capsem-cache .* prune --apply --reason"


def _holdings(command: str, *, exclusive: bool = True) -> tuple:
    return preflight.holdings(
        CONFIG,
        RecordingRunner(ROOT),
        command,
        exclusive=exclusive,
        declared=(),
        plan=gate_plan(command),
    )


@pytest.mark.parametrize("command", ("host-image", "test-functional", "pack-initrd"))
def test_every_exclusive_command_prunes_last(command: str) -> None:
    holdings = _holdings(command)
    assert isinstance(holdings[0], MachineCacheBound), (
        f"{command} must hold routine retention outside everything else, so it "
        "runs after the last step and every other teardown"
    )


def test_a_command_without_the_machine_never_mutates_the_cache() -> None:
    held_resources = _holdings("test-release-contracts", exclusive=False)
    assert MachineCacheBound not in [type(resource) for resource in held_resources]


def _acting() -> RecordingRunner:
    runner = RecordingRunner(ROOT)
    runner.observing = False
    return runner


def test_retention_runs_after_the_last_step_only() -> None:
    runner = _acting()
    with held(MachineCacheBound(runner)):
        assert not runner.matching(PRUNE), "retention must not delay the first step"
    assert len(runner.matching(PRUNE)) == 1


def test_a_failed_plan_still_prunes() -> None:
    runner = _acting()
    with pytest.raises(RuntimeError), held(MachineCacheBound(runner)):
        raise RuntimeError("the plan failed")
    assert len(runner.matching(PRUNE)) == 1


def test_a_dry_run_prunes_nothing() -> None:
    runner = RecordingRunner(ROOT)
    runner.observing = True
    with held(MachineCacheBound(runner)):
        pass
    assert not runner.matching(PRUNE)
