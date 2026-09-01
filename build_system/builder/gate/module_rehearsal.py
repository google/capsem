"""The release lane's pulled path, run locally against what the gate just built.

`AGENTS.md` says a release lane "does not run a different gate". That was true
of fifteen of the binary lane's twenty steps and false of the other five, and
the five were the ones nobody could run: verifying a digest-selected cohort,
and proving the publishable package against it. `just test` filled those slots
by *building* instead -- `artifacts.build-chain` where a release verifies, and
`glowup.install` where a release proves a pulled package.

The cost of that was measured. Seven binary-release dispatches, forty minutes
each, every one failing in a step no local run reaches: a profile axis reading
the checkout, a glow-up composing paths from the checkout layout, suites handed
no content selection, a workspace `pnpm install` reaching a registry inside a
loopback-only namespace, a gitignored generated file nothing produced. The
last one found before this module existed was worse still -- the two glow-up
steps passed none of the script's three required arguments, so they could not
have started at all.

So the cohort is fabricated from what the local lane built, verified by the
release lane's digest checker, checked at the staged path boundary, and handed
to the same `pulled_package` sequence. The candidate's immediately preceding
functional phase already booted and tested those exact bytes. Repeating every
suite after copying them was a warm second run, not independent evidence.

Hosted release lanes still execute `functional` themselves because they do not
inherit local evidence. This module shares only inside one source run, after a
fresh behavioral result and a manifest-digest proof are both dependencies.

This is not a shortcut around the release, and it does not become one. It runs
only in the local lane -- a release *is* the pulled path, and rehearsing it
inside itself would double an hour of work to prove something it is already
proving. Nothing here publishes: the cohort's URLs are `file://` paths under
`cache/target/`, and no step in this phase reaches a network or a tag.
"""

from __future__ import annotations

from pathlib import Path

from . import qualification as qualification_state
from .actions import Script
from .command import GateCommand
from .config import GateConfig
from .content import ProfileContent
from .execution import Kind, Needs, Speed, Step, step
from .module_artifacts import pulled_artifacts
from .module_glowup import pulled_package
from .plan import Plan
from .profileaxis import AxisAgrees
from .qualification import Qualification
from .testmodules import InWorkspace
from .versions import workspace_version

PHASE = "rehearsal"


class RehearsalModule(
    InWorkspace,
    GateCommand,
    name="test-rehearsal",
    help="replay the release lane's pulled path against what this tree built",
):
    """The release lane's pulled path, as a command of its own.

    Its own command for the reason every other phase has one: so it can be run
    without the hour in front of it. Composed into `candidate` it sits after the
    glow-up, which means a defect in it costs a complete gate run to see -- and
    six were found that way, at two hours and twenty minutes each. That is the
    cost structure this module exists to remove from CI, reproduced one level
    down, and the fix is the same one: make the expensive thing runnable early.

    Needs a tree that has already built its assets and packages, so it is not
    part of the fast lane. Point it at a prefix that has them with `--prefix`.
    """

    uses_qualification = True

    def plan(self) -> Plan:
        plan = Plan(self.name)
        rehearsal(plan, self._config, qualification=self.qualification)
        return plan


def rehearsal(
    plan: Plan,
    config: GateConfig,
    *,
    qualification: Qualification,
    after: tuple[Step, ...] = (),
) -> Step:
    """Replay the release lane's pulled steps against a local cohort.

    The candidate's functional phase has already proved these exact bytes. The
    release-input verifier below binds the restaged paths to their digests, so
    repeating pytest, injection, integration, and timing against the copy would
    be a warm second measurement rather than another behavioral cohort.
    """
    if qualification.pulled:
        # Already the subject. A release proves this path for real, and a
        # rehearsal beside it would be the same work done twice.
        return after[-1]

    settings = config.modules
    phase = plan.phase(PHASE)
    package = settings.rehearsal_package.format(
        version=workspace_version(config.root), arch=config.host_arch().dpkg
    )
    built = phase.add(
        step(
            "cohort",
            Script(
                config,
                settings.rehearsal_script,
                "--assets-dir",
                config.path(config.assets.test_root)
                / config.suites.pytest.base_profile
                / config.assets.merged_assets_dir,
                "--bin-dir",
                settings.default_bin_dir,
                "--packages-dir",
                config.outputs.packages,
                "--work-dir",
                settings.rehearsal_work_dir,
                "--inputs-dir",
                settings.rehearsal_inputs_dir,
                "--package",
                package,
                "--content-root",
                settings.rehearsal_content_root,
                "--before-inputs",
                settings.rehearsal_before_inputs,
                "--channel",
                settings.rehearsal_channel,
            ),
            kind=Kind.STATIC_TEST,
            needs=frozenset({Needs.DISK}),
            speed=Speed.FAST,
        ),
        after=after,
    )

    # The same content the local glow-up proved, re-derived through the release
    # lane's own staging rather than read where the build left it. That is the
    # point: three separate defects were a consumer resolving this cohort from
    # the checkout layout, and none of them was reachable while the only local
    # path built the layout it then read.
    verified = pulled_artifacts(
        plan,
        config,
        input_dir=settings.rehearsal_inputs_dir,
        profile=None,
        after=(built,),
        phase_name=PHASE,
    )
    rehearsed = qualification_state.rehearsal(
        config,
        input_dir=settings.rehearsal_inputs_dir,
        package=package,
    )
    staged = ProfileContent.staged(config, config.path(settings.rehearsal_content_root))

    # Path selection is still proved after staging: this is the cheap boundary
    # that caught checkout-relative release defects, without rebooting bytes
    # whose manifest digest and fresh behavioral result are already in this
    # plan's dependency chain.
    proved = phase.add(
        step(
            "axis",
            AxisAgrees(assets=staged.assets, profiles_dir=staged.profiles(config)),
            kind=Kind.UNIT_TEST,
            needs=frozenset({Needs.DISK}),
            speed=Speed.FAST,
        ),
        after=(verified,),
    )
    return pulled_package(
        phase,
        config,
        rehearsed,
        (proved,),
        staged,
        work_dir=settings.rehearsal_glowup_work_dir,
        # The install half owns the machine: it purges the host's `capsem`,
        # deletes `~/.capsem`, and reinstalls from the channel under test. A
        # release runner is disposable and a developer's machine is not, and
        # `just test` already installs the package it built inside the install
        # container. What is rehearsed here is the assembly the release lane
        # does from a pulled cohort, which is where the defects were.
        skip_install=True,
        # The transition, which is the deepest thing a release lane does and the
        # last thing here that had no local counterpart. `auto` classifies from
        # the two manifests, and the before-state the cohort fabricates is an
        # unpublished channel -- so this rehearses FRESH_INSTALL, the pairing a
        # first release makes. No before-package is passed, and none may be:
        # `resolve_public_before_package` refuses one for a channel that has
        # published nothing, which is exactly the case being proved.
        pairing=settings.release_pairing.runtime(
            channel=settings.rehearsal_channel,
            baseline_channel=settings.rehearsal_channel,
            transition="auto",
            before_manifest=Path(settings.rehearsal_before_inputs) / config.install.manifest_name,
            after_manifest=settings.rehearsal_after_manifest.format(
                channel=settings.rehearsal_channel
            ),
            before_profile_inputs=settings.rehearsal_before_inputs,
            after_profile_inputs=settings.rehearsal_inputs_dir,
        ),
    )
