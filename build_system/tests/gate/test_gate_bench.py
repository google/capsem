"""`just bench` exists, and it measures on config-owned terms.

There was no entry point at all. Nine Criterion bench targets existed and
nothing ran them; the guest modules were reachable only from inside a VM
suite; and a release qualification failed on a gateway CPU figure -- 0.160s
against a 0.140s budget -- that no run had ever recorded, so the number could
not be argued with. A rerun showed it was a one-off.

What this pins is the two things a plan is for, plus the one thing the gate
contract requires of every value in it.
"""

from __future__ import annotations

import argparse
import shlex
from pathlib import Path

import pytest
from capsem_builder.gate import bench
from capsem_builder.gate import config as gate_config
from helpers.gate import RecordingRunner

PROJECT_ROOT = Path(__file__).resolve().parents[3]
CONFIG = gate_config.load(PROJECT_ROOT)
SETTINGS = CONFIG.benchmark.run


def test_collectors_live_under_benchmark_ownership() -> None:
    """Executable inputs belong beside the reviewed benchmark evidence."""
    assert SETTINGS.collectors == "benchmarks/collectors"
    assert (PROJECT_ROOT / SETTINGS.collectors).is_dir()


def _plan(cls, **overrides):
    args = argparse.Namespace(
        dry_run=False,
        graph=False,
        timing=False,
        dimensions="",
        quick=False,
        commit="unknown",
    )
    for key, value in overrides.items():
        setattr(args, key, value)
    return cls(RecordingRunner(PROJECT_ROOT), args).plan()


def _argv(plan, label: str) -> list[str]:
    """What `--dry-run` prints for one step, back as arguments.

    Read through `render()` rather than the action's internals, because that
    string is the promise: the gate contract requires a dry run to be
    concrete enough to run by hand, so a test that agrees with it is testing
    what a reader will see.
    """
    return shlex.split(_step(plan, label).actions[0].render())


def test_the_harness_is_built_before_it_is_invoked() -> None:
    """Its absence otherwise reads as a benchmark failure and is not one.

    `bench.build` produces the binary `bench.run` names, and the graph orders
    them -- declared, not written one `plan.add` above the other.
    """
    plan = _plan(bench.BenchCommand)
    labels = [step.label for step in plan.steps]
    assert labels.index("prepare.bench.build") < labels.index("bench.run")

    produced = {str(path) for path in _step(plan, "prepare.bench.build").produces}
    assert str(CONFIG.path(SETTINGS.binary)) in produced
    assert _argv(plan, "bench.run")[0] == str(CONFIG.path(SETTINGS.binary))


def test_benchmark_owner_prepares_every_collector_input() -> None:
    """Focus and direct benchmark runs must not inherit a warm checkout."""
    plan = _plan(bench.BenchCommand)
    labels = [step.label for step in plan.steps]

    for label in (
        "prepare.materialize-config",
        "prepare.bench.build",
        "prepare.bench.sign",
    ):
        assert labels.index(label) < labels.index("bench.run")
    assert any(label.startswith("initrd.") for label in labels)


def test_quick_benchmark_prepares_host_inputs_without_guest_work() -> None:
    """The quick lane needs profiles and binaries but keeps its no-guest promise."""
    labels = [step.label for step in _plan(bench.BenchCommand, quick=True).steps]

    assert "prepare.materialize-config" in labels
    assert "prepare.bench.build" in labels
    assert "prepare.bench.sign" in labels
    assert not any(label.startswith(("assets.", "initrd.")) for label in labels)


def test_complete_gate_fitness_uses_the_owned_harness() -> None:
    """Preparation asks the Rust machine model before expensive VM work."""
    harness, fitness = bench.fitness(CONFIG)
    build = [shlex.split(action.render()) for action in harness.actions]
    doctor = [shlex.split(action.render()) for action in fitness.actions]

    assert build[0] == [
        "cargo",
        "build",
        "-p",
        SETTINGS.crate,
        "--bin",
        SETTINGS.bin_name,
    ]
    # Standing only: the gate's own compile precedes this step, and its load
    # refused three candidates on an otherwise quiet machine. `run` judges
    # load when it measures.
    assert doctor[0] == [str(CONFIG.path(SETTINGS.binary)), "doctor", "--standing"]
    assert [exclusive.name for exclusive in harness.contends] == ["workspace_binaries"]
    assert [exclusive.name for exclusive in fitness.contends] == ["host_service"]


def _step(plan, label: str):
    for step in plan.steps:
        if step.label == label:
            return step
    raise AssertionError(f"no step {label!r}")


def test_measuring_holds_the_machine() -> None:
    """A benchmark sharing a CPU with the rest of a gate measures the sharing.

    This is the single reason the step declares contention: two dimensions
    running at once would each be the other's noise.
    """
    plan = _plan(bench.BenchCommand)
    assert _step(plan, "bench.run").contends


def test_the_quick_lane_is_bounded_by_its_own_promise() -> None:
    """`bench-quick` is only useful while it stays a dev loop.

    The ceiling is config-owned and separate from the full run's, so making
    the full run more patient cannot quietly make the dev loop slower.
    """
    plan = _plan(bench.BenchCommand, quick=True)
    argv = _argv(plan, "bench.quick")
    assert "--quick" in argv
    assert str(SETTINGS.quick_timeout_secs) in argv
    assert SETTINGS.quick_timeout_secs < SETTINGS.timeout_secs


def test_a_collector_is_always_bounded() -> None:
    """One that never exits would hold the machine lock the gate runs under."""
    argv = _argv(_plan(bench.BenchCommand), "bench.run")
    assert "--timeout-secs" in argv
    assert argv[argv.index("--timeout-secs") + 1] == str(SETTINGS.timeout_secs)


def test_named_dimensions_reach_the_harness() -> None:
    """One quoted argument in, separate dimensions out.

    The recipe interpolates through `{{quote(...)}}` because double quotes
    still expand `$(...)` and backticks -- `just bench "x; echo pwned"` ran
    the echo. So the payload arrives joined and is split here, where no shell
    is involved.
    """
    argv = _argv(_plan(bench.BenchCommand, dimensions="routes criterion"), "bench.run")
    assert argv[-2:] == ["routes", "criterion"]


def test_a_dimension_name_cannot_reach_a_shell() -> None:
    """The payload is words to the plan, never a command line."""
    hostile = "routes; echo pwned"
    argv = _argv(_plan(bench.BenchCommand, dimensions=hostile), "bench.run")
    assert argv[-3:] == ["routes;", "echo", "pwned"]
    assert "bench" not in argv[-3:]


@pytest.mark.parametrize(
    "value",
    [
        SETTINGS.collectors,
        SETTINGS.store,
        SETTINGS.interpreter,
        SETTINGS.crate,
        SETTINGS.bin_name,
    ],
)
def test_every_value_comes_from_config(value: str) -> None:
    """The gate contract: no path, filename or name literal in a plan module.

    Each of these was a `default_value` in the binary's own CLI, which is fine
    for running it by hand and is not config -- the gate passes them in.
    """
    source = (PROJECT_ROOT / "build_system" / "builder" / "gate" / "bench.py").read_text(encoding="utf-8")
    assert f'"{value}"' not in source


def test_the_report_reads_the_store_the_run_wrote() -> None:
    """Two commands, one store, or the report describes nothing."""
    run = _argv(_plan(bench.BenchCommand), "bench.run")
    report = _argv(_plan(bench.BenchReportCommand), "bench.report")
    store = str(CONFIG.path(SETTINGS.store))
    assert store in run and store in report


class _ActingRunner(RecordingRunner):
    """Records commands but acts on the filesystem, as a real run does."""

    observing = False


def _linked_worktree(tmp_path: Path):
    """A checkout whose shared build root lives elsewhere, as in a linked worktree."""
    checkout = tmp_path / "worktree"
    checkout.mkdir(parents=True)
    # The compiler cache resource reads the checkout's cache policy.
    (checkout / "config").symlink_to(PROJECT_ROOT / "config", target_is_directory=True)
    return CONFIG.model_copy(
        update={
            "root": checkout,
            "prefix": CONFIG.prefix.model_copy(update={"cargo_target": str(tmp_path / "shared" / "cargo")}),
        }
    )


def test_a_linked_worktree_reads_binaries_where_cargo_writes_them(tmp_path: Path) -> None:
    """`just bench` built into the shared target and then hashed the checkout's.

    Every exclusive command in a linked worktree compiled into the shared build
    root while its steps named `cache/target/cargo/debug/...` under the checkout,
    so `prepare.bench.build` failed with "cannot hash ... it is not a file"
    before anything was measured. The checkout's profile directories must
    resolve to the build root the compiler was given, as a prefix's do.
    """
    from capsem_builder.gate import cargotarget, preflight

    config = _linked_worktree(tmp_path)
    shared = cargotarget.path(config)
    (shared / "debug").mkdir(parents=True)
    (shared / "debug" / "capsem").write_bytes(b"built")
    resources = preflight.holdings(config, _ActingRunner(config.root), "measure", exclusive=True, declared=())
    link = next(resource for resource in resources if isinstance(resource, cargotarget.CheckoutBuildRoot))
    link.acquire()
    debug = config.root / "cache" / "target" / "cargo" / "debug"
    assert debug.is_symlink() and (debug / "capsem").read_bytes() == b"built"
    link.acquire()  # idempotent on the next command
    assert debug.readlink() == shared / "debug"


def test_the_checkout_that_owns_the_build_root_and_observers_are_left_alone(tmp_path: Path) -> None:
    from capsem_builder.gate import cargotarget

    owner = CONFIG.model_copy(
        update={
            "root": tmp_path,
            "prefix": CONFIG.prefix.model_copy(update={"cargo_target": str(tmp_path / "cache" / "target" / "cargo")}),
        }
    )
    cargotarget.CheckoutBuildRoot(owner, _ActingRunner(tmp_path)).acquire()
    assert not (tmp_path / "cache" / "target" / "cargo" / "debug").exists()

    observed = _linked_worktree(tmp_path / "observed")
    cargotarget.CheckoutBuildRoot(observed, RecordingRunner(observed.root)).acquire()
    assert not (observed.root / "cache").exists(), "interrogating a plan must not touch the checkout"


def test_a_real_profile_directory_in_a_worktree_is_named_not_overwritten(tmp_path: Path) -> None:
    from capsem_builder.gate import cargotarget
    from capsem_builder.gate.errors import GateError

    config = _linked_worktree(tmp_path)
    stale = config.root / "cache" / "target" / "cargo" / "debug"
    stale.mkdir(parents=True)
    (stale / "old").write_bytes(b"x")
    with pytest.raises(GateError, match="real directory"):
        cargotarget.CheckoutBuildRoot(config, _ActingRunner(config.root)).acquire()
    assert (stale / "old").exists()
