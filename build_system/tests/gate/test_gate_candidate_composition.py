"""`just test-full` is one process, one lock, one workspace, one plan.

It used to be a tree of processes. `candidate` ran `just _test-fast`, which ran
`capsem-gate test-fast`; then a Colima wrapper around `just _test-candidate`,
which ran `capsem-gate test-candidate`, which ran four more `capsem-gate`
commands, each of which ran several more. Every one of those is exclusive, and
the machine lock is not reentrant, so the whole shape was a queue of children
waiting out a 7200-second timeout for a lock their own parent was holding.

Composed, the ordering that used to be spread across recipe dependencies, shell
line order and four nested plans becomes edges in one graph -- and three things
that were implicit become structural.

The Colima lifecycle was a shell trap in `with-gate-colima.sh`; it is a
`Resource`, because "restore what I found on the way out" is exactly what a
resource is. The orphan-process accounting was a `finally`, and has to run on
every path including the aborted one -- which a step cannot do, since a step
whose dependency failed is skipped. It is a resource too. And the source state
is recorded by a step and re-asserted by another, rather than captured while
the plan was being built.
"""

from __future__ import annotations

import argparse
from pathlib import Path

import pytest
from capsem_builder.gate import cli  # noqa: F401 - imported so every command registers
from capsem_builder.gate import config as gate_config
from capsem_builder.gate.command import GateCommand
from capsem_builder.gate.resume import ancestors
from helpers.gate import RecordingRunner

PROJECT_ROOT = Path(__file__).resolve().parents[3]


#: `resources()` takes the runner it should build with; these tests ask
#: *what* is held, so any runner will do.
def _resource_runner():
    from helpers.gate import RecordingRunner

    return RecordingRunner(PROJECT_ROOT)


RUNNER_FOR_RESOURCES = _resource_runner()
CONFIG = gate_config.load(PROJECT_ROOT)


def _candidate():
    return GateCommand.registry["candidate"](
        RecordingRunner(PROJECT_ROOT),
        argparse.Namespace(dry_run=False, graph=False, timing=False),
    )


def _plan():
    return _candidate()._describe()


def _at(labels: list[str], prefix: str) -> int:
    """Where a phase sits, by its first step."""
    for position, label in enumerate(labels):
        if label.startswith(prefix):
            return position
    raise AssertionError(f"no step starting {prefix!r} in:\n  " + "\n  ".join(labels))


# ---------------------------------------------------------------------------
# One plan, not a tree of processes
# ---------------------------------------------------------------------------


def test_the_whole_gate_is_one_plan() -> None:
    """Every phase is in it, so one run log and one lock cover all of them."""
    labels = list(_plan().labels)

    for phase in ("fast.", "static.", "artifacts.", "functional.", "glowup."):
        _at(labels, phase)


def test_linux_signing_steps_preserve_the_graph_without_launching_apple_tools(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Linux still follows the same dependencies, but never execs codesign."""
    from capsem_builder.gate import host

    monkeypatch.setattr(host, "on_macos", lambda: False)
    signing = [step for step in _plan().steps if step.label.endswith(".sign")]

    assert signing
    assert all(not step.actions for step in signing)
    assert all(not step.produces for step in signing)


def test_macos_signing_step_keeps_codesign_and_artifact_ownership(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    from capsem_builder.gate import host, hostpackage

    monkeypatch.setattr(host, "on_macos", lambda: True)
    signing = hostpackage.sign_step(CONFIG)

    assert signing.actions
    assert all("codesign" in action.render() for action in signing.actions)
    assert signing.produces == tuple(CONFIG.path(path) for path in CONFIG.signing.binaries)


def test_local_package_rails_defer_to_the_authoritative_install_transaction() -> None:
    """The complete gate must not need a mutable public channel to recover one.

    Its final install transaction authors a checked local release graph before
    installing the exact native package.  Running the narrower package proof
    first would hydrate from public stable and make a broken channel impossible
    to repair through the only supported release commands.
    """
    plan = _plan()

    for arch in CONFIG.architectures:
        rendered = "\n".join(plan.step_named(f"package.{arch}.prove").render())
        assert "defer exact package proof to the local install transaction" in rendered

    installed = "\n".join(plan.step_named("glowup.install").render())
    assert "install the exact package and prove the installed product" in installed


def test_the_phases_run_in_the_order_the_gate_depends_on() -> None:
    """Cheap failures first, and nothing that needs artifacts before they exist.

    This order used to live in three languages at once: `just` dependencies,
    the line order of a shell body, and the sequence of `plan.add` calls in
    four separate commands.
    """
    labels = list(_plan().labels)

    assert _at(labels, "fast.") < _at(labels, "static.")
    assert _at(labels, "static.") < _at(labels, "artifacts.")
    assert _at(labels, "artifacts.") < _at(labels, "functional.")
    assert _at(labels, "functional.") < _at(labels, "glowup.")


def test_every_local_functional_vm_step_selects_its_exact_ironbank_profile() -> None:
    """Diagnostic continuation may start at any VM step, so selection travels
    with every step rather than relying on one earlier mutable selector."""
    from capsem_builder.gate import profiles as gate_profiles

    plan = _plan()
    names = CONFIG.environment

    for profile in gate_profiles.selected(CONFIG):
        assets = CONFIG.path(CONFIG.assets.test_root) / profile / CONFIG.assets.merged_assets_dir
        profiles = (
            CONFIG.path(CONFIG.assets.test_root)
            / profile
            / CONFIG.assets.merged_config_dir
            / CONFIG.assets.materialized_profiles_dir
        )
        labels = [
            label
            for label in plan.labels
            if label.startswith("functional.") and label.endswith(f".{profile}")
        ]
        assert labels
        for label in labels:
            rendered = "\n".join(plan.step_named(label).render())
            if ".pytest." in label:
                assert f"{names.assets_dir}={assets}" in rendered, label
                assert f"{names.profiles_dir}={profiles}" in rendered, label
            else:
                assert f"--assets {assets}" in rendered, label
                if ".injection." in label:
                    assert f"--profiles-dir {profiles}" in rendered, label
                else:
                    assert f"{names.profiles_dir}={profiles}" in rendered, label


def test_the_source_state_is_recorded_first_and_re_asserted_last() -> None:
    """A gate that qualified a HEAD nobody has proved nothing about anything."""
    labels = list(_plan().labels)

    assert labels.index("source.record") == 0
    assert labels.index("source.verify") == len(labels) - 1


def test_the_fast_phase_precedes_everything_expensive() -> None:
    """It is the most common failure class and the cheapest to reach."""
    labels = list(_plan().labels)

    assert _at(labels, "fast.") < _at(labels, "prepare")


def test_preparation_waits_for_every_fast_leaf_and_not_one_incidental_step() -> None:
    """A phase is finished when every independent branch of it is finished.

    `fast()` returned Clippy, because Clippy happened to be added last. Clippy
    waits on the Rust toolchain and one web surface and on nothing else, so
    everything the phase exists to front-load -- Ruff, both Ty passes, the
    dependency audits, the other three web surfaces -- was free to still be
    running while the gate built assets and booted VMs. The contract is that
    the cheap failures come *before* the expensive work, and for most of them
    it was not true.

    `sourcechecks.fragment` had already learned this one level down and says so
    in its own docstring; the caller threw its answer away.
    """
    plan = _plan()
    first_prepare = next(label for label in plan.labels if label.startswith("prepare."))
    leaves = {label for label in plan.labels if label.startswith(("fast.", "python."))}

    assert leaves <= ancestors(plan, first_prepare), (
        "these run in the fast phase and gate nothing: "
        + ", ".join(sorted(leaves - ancestors(plan, first_prepare)))
    )


def test_shared_host_image_waits_for_canonical_preparation() -> None:
    """Sealing the gate must not race Docker work ahead of bootstrap.

    The install-image preflight was correctly ordered after preparation, but
    the shared host image it derives from was not.  It therefore started in
    the first wave, before both the cheap checks and canonical bootstrap, and
    tried to resolve a registry from the gate's no-egress namespace.
    """
    plan = _plan()
    prerequisites = ancestors(plan, "host-image")
    fast = {label for label in plan.labels if label.startswith(("fast.", "python."))}

    assert "prepare.bootstrap" in prerequisites
    assert fast <= prerequisites, "these fast checks can still race the host image: " + ", ".join(
        sorted(fast - prerequisites)
    )


# ---------------------------------------------------------------------------
# What must happen even when the gate fails
# ---------------------------------------------------------------------------


def test_the_things_that_must_survive_a_failure_are_resources() -> None:
    """A step whose dependency failed is skipped, which is wrong for these two.

    An aborted run is exactly the run that skips its own cleanup, so it is
    exactly the run whose surviving processes need counting -- sixteen
    `capsem-service` processes each holding a tray once accumulated in a day
    while every run reported success. And a Colima the gate started must stop
    whether or not the gate passed.
    """
    names = [resource.name for resource in _candidate().resources(RUNNER_FOR_RESOURCES)]

    assert "orphan-accounting" in names
    assert "colima" in names


def test_the_orphan_baseline_is_taken_before_anything_can_spawn_a_process() -> None:
    """Otherwise a developer's own dev daemon is blamed on this run.

    Acquisition order is the guarantee: resources are acquired in order and
    released in reverse, so the baseline is first taken and last compared.
    """
    names = [resource.name for resource in _candidate().resources(RUNNER_FOR_RESOURCES)]

    assert names[0] == "orphan-accounting"


def test_the_workspace_is_held_for_the_whole_gate() -> None:
    names = [resource.name for resource in _candidate().resources(RUNNER_FOR_RESOURCES)]

    assert "workspace" in names


# ---------------------------------------------------------------------------
# What composition made checkable
# ---------------------------------------------------------------------------


def test_nothing_in_the_plan_starts_another_gate() -> None:
    """The property the whole change exists for, asserted on the real plan."""
    from capsem_builder.gate.funnel import ENTRYPOINTS, program

    offences = [
        f"{label}: {rendered}"
        for label in _plan().labels
        for rendered in _plan().step_named(label).render()
        for argv in [rendered.split()]
        if argv and program(tuple(argv)) in ENTRYPOINTS
    ]

    assert not offences, "the composed gate still launches a gate:\n  " + "\n  ".join(offences)


def test_the_plan_is_acyclic_and_every_exclusive_it_claims_is_declared() -> None:
    """Both are checked before the machine lock is taken, and cost nothing."""
    _plan().validate(CONFIG)


@pytest.mark.parametrize(
    "exclusive", ["apple_vz", "docker_daemon", "workspace_binaries", "host_service"]
)
def test_the_composed_gate_still_declares_what_may_not_overlap(exclusive: str) -> None:
    """Four commands each had their own machine lock making this true by
    accident. One plan has to say it."""
    claimed = {
        resource.name for label in _plan().labels for resource in _plan().step_named(label).contends
    }

    assert exclusive in claimed
