"""Citadel guard: nothing runs in a release that has not run locally first.

`AGENTS.md` says a release lane "does not run a different gate". It ran the
same one for fifteen of its twenty steps. The other five -- verifying a
digest-selected cohort, and the four that prove the publishable package against
it -- had no local counterpart at all, because `just test-clean` filled those slots by
building instead.

Seven binary-release dispatches were spent finding defects in them, forty
minutes each, and the last of those defects was that two of the five passed
none of their script's three required arguments. They could never have started.
Nothing said so, because nothing compared the two plans.

This does. Coverage is compared by label: a release step named `<phase>.<rest>`
is covered locally either by the identical label or by `rehearsal.<rest>`.
Anything else is a step that only a release dispatch will ever run.

Idleness is compared by rendered command instead, because the two questions are
not the same one. A step can share its name with a candidate step and still be
the only local proof of anything -- the VM suites read a staged cohort under a
pulled qualification where the candidate reads the layout it just built. Three
more dispatches went to defects in that difference after the five nameless
steps were covered.
"""

from __future__ import annotations

from pathlib import Path

from capsem_builder.gate import config as gate_config
from capsem_builder.gate.qualification import from_environment
from helpers.gate import built_command

ROOT = Path(__file__).resolve().parents[2]

#: A staged workspace that is not, and cannot be confused with, the checkout.
STAGED = Path("/staged-release-workspace")

REHEARSAL = "rehearsal"


def _labels(name: str, arguments: tuple, qualification) -> tuple[str, ...]:
    return tuple(built_command(ROOT, name, arguments, qualification)._describe().labels)


def _plans() -> tuple[tuple[str, ...], tuple[str, ...]]:
    """The local gate's steps, and the binary release lane's."""
    config = gate_config.load(ROOT)
    settings = config.modules
    release = from_environment(
        config,
        {
            settings.release_input_dir: str(STAGED / "target/candidate-profile-inputs"),
            settings.release_package: str(STAGED / "release-test-package/capsem.deb"),
            settings.release_bin_dir: str(STAGED / "target/debug"),
        },
    )
    local = _labels("candidate", (), from_environment(config, {}))
    pulled = _labels("qualify-binaries", (("workspace_root", STAGED),), release)
    return local, pulled


def test_every_release_step_has_a_local_counterpart() -> None:
    """The whole point: a green `just test-clean` has to mean something about CI."""
    local, pulled = _plans()
    covered = set(local)

    # Matched by what the step does rather than by which phase does it. A
    # release lane makes the generated settings in its functional phase because
    # that is the only phase it has; the local gate makes them once in the fast
    # phase and hands the step on. Same work, same command, different label --
    # and a guard that insisted on the phase would report that as a blind spot.
    suffixes = {label.split(".", 1)[-1] for label in covered}
    uncovered = [
        label
        for label in pulled
        if label not in covered and label.split(".", 1)[-1] not in suffixes
    ]

    assert not uncovered, (
        "these steps run only in a binary release, so the only way to find a "
        "defect in one is to dispatch a release and wait: " + ", ".join(sorted(uncovered))
    )


def test_the_rehearsal_covers_exactly_the_steps_that_differ() -> None:
    """A rehearsal step with nothing to rehearse is a step proving itself.

    The guard above is satisfied by adding rehearsal steps; this one keeps them
    honest: a `rehearsal.<rest>` that duplicates work the candidate already does
    buys nothing and costs the run.

    Compared by what the step *runs*, not by what it is called. That distinction
    is the whole reason the VM suites are rehearsed at all. `functional` and
    `rehearsal` both hold a step named `pytest.broad.code`, and by label the
    second looks redundant -- but the first reads the layout the build left
    behind and the second reads a cohort staged the way a release stages one,
    under a qualification that reports itself as pulled. Four of the eight
    binary-release failures were in exactly that gap, each dying within four
    seconds on a precondition the local run had already satisfied by building
    it. A guard that matched on the name alone would have called every one of
    those steps idle and sent the defects back to the dispatch queue.
    """
    local_plan = built_command(ROOT, "candidate", (), None)._describe()
    commands: dict[str, set[str]] = {}
    for step in local_plan.steps:
        commands[step.label] = {action.render() for action in step.actions}

    elsewhere = {
        rendered
        for label, rendered_set in commands.items()
        if not label.startswith(f"{REHEARSAL}.")
        for rendered in rendered_set
    }

    idle = sorted(
        label
        for label, rendered_set in commands.items()
        if label.startswith(f"{REHEARSAL}.")
        # The cohort step is the fabricator itself: it has no release
        # counterpart because a release is handed its cohort rather than
        # building one.
        if label != f"{REHEARSAL}.cohort"
        if rendered_set and rendered_set <= elsewhere
    )

    assert not idle, (
        f"{idle} run commands the candidate already runs verbatim elsewhere, so "
        "they add time without adding coverage"
    )


def test_only_the_rehearsal_skips_the_installed_transition() -> None:
    """`--skip-install` is what makes a rehearsal safe, and a release unproven.

    The install half purges the host `capsem`, deletes `~/.capsem`, and installs
    from the channel under test. That is correct on a disposable runner and not
    something `just test-clean` may do to a machine somebody is working on -- so the
    rehearsal skips it and the release must not, which is exactly the pair of
    claims worth pinning in one place.
    """
    local_plan = built_command(ROOT, "candidate", (), None)._describe()
    config = gate_config.load(ROOT)
    settings = config.modules
    release_plan = built_command(
        ROOT,
        "qualify-binaries",
        (("workspace_root", STAGED),),
        from_environment(
            config,
            {
                settings.release_input_dir: str(STAGED / "target/candidate-profile-inputs"),
                settings.release_package: str(STAGED / "release-test-package/capsem.deb"),
                settings.release_bin_dir: str(STAGED / "target/debug"),
            },
        ),
    )._describe()

    def skipping(plan) -> set[str]:
        return {
            step.label
            for step in plan.steps
            for action in step.actions
            if "--skip-install" in action.render()
        }

    rehearsed = skipping(local_plan)
    assert rehearsed, "the rehearsal installs a pulled package on the host machine"
    assert all(label.startswith(f"{REHEARSAL}.") for label in rehearsed), (
        f"{sorted(rehearsed)} skip the installed transition outside the rehearsal"
    )
    released = skipping(release_plan)
    assert not released, (
        f"{sorted(released)} skip the installed transition during a release, so "
        "nothing proves the package can actually be installed"
    )


def test_a_release_lane_does_not_rehearse_itself() -> None:
    """There it is not a rehearsal -- it is the lane.

    Running both would spend the pulled glow-up's quarter-hour twice to prove
    the same sequence, once against the cohort that ships and once against one
    fabricated beside it.
    """
    _, pulled = _plans()
    doubled = [label for label in pulled if label.startswith(f"{REHEARSAL}.")]
    assert not doubled, f"the release lane rehearses itself at {doubled}"


def test_the_rehearsal_publishes_nothing_and_fetches_nothing() -> None:
    """A local cohort is local. Its URLs are `file://` under `target/`.

    Not a stylistic rule. The rehearsal exists to run the release lane's steps,
    and the steps around it in a real lane do publish -- so a rehearsal that
    grew a fetch or an upload would be a `just test-clean` that touches a channel.
    """
    local_plan = built_command(ROOT, "candidate", (), None)._describe()
    reaching = [
        f"{step.label}: {rendered}"
        for step in local_plan.steps
        if step.label.startswith(f"{REHEARSAL}.")
        for action in step.actions
        if (rendered := action.render())
        if "https://" in rendered or "http://" in rendered or rendered.startswith("gh ")
    ]
    assert not reaching, "the rehearsal reaches outside this machine:\n  " + "\n  ".join(reaching)
