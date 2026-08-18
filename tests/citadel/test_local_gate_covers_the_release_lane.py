"""Citadel guard: nothing runs in a release that has not run locally first.

`AGENTS.md` says a release lane "does not run a different gate". It ran the
same one for fifteen of its twenty steps. The other five -- verifying a
digest-selected cohort, and the four that prove the publishable package against
it -- had no local counterpart at all, because `just test` filled those slots by
building instead.

Seven binary-release dispatches were spent finding defects in them, forty
minutes each, and the last of those defects was that two of the five passed
none of their script's three required arguments. They could never have started.
Nothing said so, because nothing compared the two plans.

This does. The comparison is by label, in both directions, and the mapping is
one rule: a release step named `<phase>.<rest>` is covered locally either by
the identical label or by `rehearsal.<rest>`. Anything else is a step that only
a release dispatch will ever run.
"""

from __future__ import annotations

from pathlib import Path

from helpers.gate import built_command

from capsem.gate import config as gate_config
from capsem.gate.qualification import from_environment

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
    """The whole point: a green `just test` has to mean something about CI."""
    local, pulled = _plans()
    covered = set(local)

    # Matched by what the step does rather than by which phase does it. A
    # release lane makes the generated settings in its functional phase because
    # that is the only phase it has; the local gate makes them once in the fast
    # phase and hands the step on. Same work, same command, different label --
    # and a guard that insisted on the phase would report that as a blind spot.
    suffixes = {label.split(".", 1)[-1] for label in covered}
    uncovered = [
        label for label in pulled if label not in covered and label.split(".", 1)[-1] not in suffixes
    ]

    assert not uncovered, (
        "these steps run only in a binary release, so the only way to find a "
        "defect in one is to dispatch a release and wait: " + ", ".join(sorted(uncovered))
    )


def test_the_rehearsal_covers_exactly_the_steps_that_differ() -> None:
    """A rehearsal step with nothing to rehearse is a step proving itself.

    The guard above is satisfied by adding rehearsal steps; this one keeps them
    honest. Every `rehearsal.<rest>` has to answer for some release step that
    the local plan does not already run under its own name -- otherwise the
    phase accumulates work that duplicates the candidate rather than extending
    it, and the extra quarter-hour buys nothing.
    """
    local, pulled = _plans()
    answers = {label.split(".", 1)[-1] for label in pulled if label not in set(local)}

    idle = [
        label
        for label in local
        if label.startswith(f"{REHEARSAL}.") and label.split(".", 1)[-1] not in answers
    ]
    # The cohort step is the fabricator itself: it has no release counterpart
    # because a release is handed its cohort rather than building one.
    idle = [label for label in idle if label != f"{REHEARSAL}.cohort"]

    assert not idle, (
        f"{idle} rehearse steps the local gate already runs under their own "
        "name, so they add time without adding coverage"
    )


def test_only_the_rehearsal_skips_the_installed_transition() -> None:
    """`--skip-install` is what makes a rehearsal safe, and a release unproven.

    The install half purges the host `capsem`, deletes `~/.capsem`, and installs
    from the channel under test. That is correct on a disposable runner and not
    something `just test` may do to a machine somebody is working on -- so the
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
    grew a fetch or an upload would be a `just test` that touches a channel.
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
