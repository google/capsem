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

So the cohort is fabricated from what the local lane built, and then everything
downstream is the release lane itself: the same `pulled_artifacts` verify step
and the same `pulled_package` sequence, from the same functions, differing only
in the directories they scratch in.

This is not a shortcut around the release, and it does not become one. It runs
only in the local lane -- a release *is* the pulled path, and rehearsing it
inside itself would double an hour of work to prove something it is already
proving. Nothing here publishes: the cohort's URLs are `file://` paths under
`target/`, and no step in this phase reaches a network or a tag.
"""

from __future__ import annotations

from pathlib import Path

from . import qualification as qualification_state
from .actions import Script
from .config import GateConfig
from .content import ProfileContent
from .execution import Kind, Needs, Speed, Step, step
from .module_artifacts import pulled_artifacts
from .module_glowup import pulled_package
from .plan import Plan
from .qualification import Qualification
from .versions import workspace_version

PHASE = "rehearsal"


def rehearsal(
    plan: Plan,
    config: GateConfig,
    *,
    qualification: Qualification,
    after: tuple[Step, ...] = (),
) -> Step:
    """Replay the release lane's pulled steps against a local cohort."""
    if qualification.pulled:
        # Already the subject. A release proves these five steps for real, and
        # a rehearsal beside them would be the same fifteen minutes spent twice.
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
                settings.rehearsal_script,
                "--assets-dir",
                config.path(config.assets.test_root)
                / config.suites.pytest.base_profile
                / config.assets.merged_assets_dir,
                "--bin-dir",
                settings.default_bin_dir,
                "--packages-dir",
                config.package.dist_dir,
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
    return pulled_package(
        phase,
        config,
        qualification_state.rehearsal(
            config,
            input_dir=settings.rehearsal_inputs_dir,
            package=package,
        ),
        (verified,),
        ProfileContent.staged(config, config.path(settings.rehearsal_content_root)),
        work_dirs=(
            settings.rehearsal_glowup_work_dir,
            settings.rehearsal_channel_switch_work_dir,
        ),
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
            transition="auto",
            before_manifest=Path(settings.rehearsal_before_inputs) / config.install.manifest_name,
            after_manifest=settings.rehearsal_after_manifest.format(
                channel=settings.rehearsal_channel
            ),
            before_profile_inputs=settings.rehearsal_before_inputs,
            after_profile_inputs=settings.rehearsal_inputs_dir,
        ),
    )
