"""The installed public-to-candidate transition, replayed before a release.

A binary release lane installs the deployed public package, waits for it to
update itself to the candidate, and then requires the updated service to come
back healthy. Nothing local ever did that: the local install proof installs
the candidate fresh, and the rehearsal pairs its cohort with a fabricated
first-release before-state and skips the install. So the stable 0.6.4
qualification found, one hosted run at a time, a profile key 0.6.3 refuses, a
rule 0.6.3 cannot reload, and a ledger 0.6.4 would not open (#280).

This fetches the deployed before-state over the release egress and replays the
transition with the release lane's own glow-up script, inside the same
disposable systemd container the install proof uses. The candidate side is the
rehearsal's cohort: the package, manifest and verified profile inputs it built
and checked. No new transition logic lives here; `local_release_glowup` owns
it, as it does in the release lane.
"""

from __future__ import annotations

from pathlib import Path

from . import config as gate_config
from .actions import Call, Script
from .config import GateConfig
from .content import ProfileContent, SelectedInstallContent
from .docker import Docker
from .execution import Kind, Needs, Speed, Step, step
from .fileactions import make_dir, remove
from .installcontainer import InstallContainer
from .opacity import CallJustification, Effect, OpaqueKind, machine_effects
from .plan import Plan
from .qualification import Qualification
from .sourcecommit import source_commit_for_checkout
from .versions import workspace_version

PHASE = "transition"
PACKAGES = "packages"
TRANSITION_EVIDENCE = "transition"
PROFILES = "profiles"


def fetch_before(config: GateConfig) -> list[Step]:
    """The deployed public packages and profiles, digest-verified.

    Outside the sandbox, which forbids a mid-run fetch: this is a fetch, and
    the release egress is the one sanctioned route for it. The cache keys by
    digest, so an unchanged public channel costs no download.
    """
    settings = config.modules
    before = config.path(settings.transition.before_dir)

    def fetch(kind: str, *extra: str) -> Script:
        return Script(
            config,
            settings.transition.fetch_script,
            "--manifest-url",
            config.package.default_manifest_url,
            "--kind",
            kind,
            *extra,
            "--output",
            before / kind,
            "--cache-dir",
            settings.transition.input_cache,
            outside_sandbox=True,
        )

    fetched = [
        step(
            f"before-{kind}",
            action,
            kind=Kind.E2E,
            needs=frozenset({Needs.NETWORK, Needs.DISK}),
            speed=Speed.SLOW,
        )
        for kind, action in (
            (PACKAGES, fetch(PACKAGES)),
            (PROFILES, fetch(PROFILES, "--architecture", config.host_arch().name)),
        )
    ]
    fetched.append(
        step(
            "before-verify",
            Script(config, settings.verify_inputs_script, "--input-dir", before / PROFILES),
            kind=Kind.STATIC_TEST,
            needs=frozenset({Needs.DISK}),
            speed=Speed.FAST,
        )
    )
    return fetched


class TransitionGate:
    """Install the public package, then let it update itself to the candidate."""

    def __init__(self, runner, *, source_commit: str) -> None:
        self._runner = runner
        self._config = gate_config.for_root(runner.root)
        modules = self._config.modules
        self._settings = self._config.install
        self._before = self._config.path(modules.transition.before_dir)
        self._inputs = self._config.path(modules.rehearsal_inputs_dir)
        self._package = self._config.path(
            modules.rehearsal_package.format(
                version=workspace_version(self._config.root), arch=self._config.host_arch().dpkg
            )
        )
        self._after = self._config.path(
            modules.rehearsal_after_manifest.format(channel=modules.rehearsal_channel)
        )
        self._content = ProfileContent.staged(
            self._config, self._config.path(modules.rehearsal_content_root)
        )
        # The candidate's own tools, as the rehearsal and the release lane use
        # them: the glow-up authors channels with them before anything is
        # installed, so the container's /usr/bin has none yet.
        self._bin_dir = self._config.path(modules.default_bin_dir).resolve()
        self._source_commit = source_commit
        self._container = InstallContainer(
            runner,
            content=SelectedInstallContent(self._content),
            inputs=(
                self._before,
                self._inputs,
                self._package.parent,
                self._after.parent,
                self._bin_dir,
            ),
        )

    def _before_package(self) -> Path | None:
        found = sorted((self._before / PACKAGES).glob(f"*_{self._config.host_arch().dpkg}.deb"))
        return found[0] if found else None

    def run(self) -> None:
        options = self._container.runtime_options()
        if not self._container.boots_a_guest:
            self._runner.note(
                "No bootable guest here: the installed transition runs on Linux only."
            )
            return
        before_package = self._before_package()
        if before_package is None:
            # A first release has no deployed package to update from; the
            # rehearsal already proves that pairing.
            self._runner.note(
                "The public channel publishes no package yet; nothing to update from."
            )
            return
        # Under the install proof's evidence root, which the container mounts
        # at its own address; the work directory is the proof's own, owned
        # path. This container runs after that proof's container is gone.
        layout = self._settings.layout
        evidence = f"{layout.glowup_evidence}/{TRANSITION_EVIDENCE}"
        remove(self._config.path(evidence))
        make_dir(self._config.path(evidence))
        try:
            self._container.start(options=options)
            self._glowup(before_package, f"{self._settings.mount}/{evidence}")
        finally:
            self._container.return_paths()
            self._container.stop()

    def _glowup(self, before_package: Path, evidence: str) -> None:
        settings = self._settings
        pairing = self._config.modules.release_pairing
        channel = self._config.modules.rehearsal_channel
        env = {
            "XDG_RUNTIME_DIR": settings.guest_user.runtime_dir,
            "UV_PROJECT_ENVIRONMENT": settings.venv,
            pairing.channel: channel,
            pairing.baseline_channel: channel,
            pairing.transition: "auto",
            pairing.before_manifest: str(
                self._before / PROFILES / self._config.install.manifest_name
            ),
            pairing.after_manifest: str(self._after),
            pairing.before_profile_inputs: str(self._before / PROFILES),
            pairing.after_profile_inputs: str(self._inputs),
        }
        command = (
            f"{settings.venv_python} {settings.suite.glowup_script} "
            f'--input-deb "{self._package}" --before-package "{before_package}" '
            f'--bin-dir "{self._bin_dir}" '
            f'--assets-dir "{self._content.assets}" --config-root "{self._content.config}" '
            f"--work-dir {settings.layout.glowup} --package-ready "
            f'--evidence-dir "{evidence}" '
            f"--source-commit {self._source_commit} "
            f"--profile-revision-policy {settings.profile_revision_policy.value}"
        )
        Docker(self._runner).shell(
            self._container.name,
            command,
            user=settings.guest_user.name,
            cwd=settings.mount,
            env=env,
        )


def transition(
    plan: Plan, config: GateConfig, *, qualification: Qualification, after: tuple[Step, ...]
) -> Step:
    """Fetch the deployed before-state, then replay the installed transition.

    Its own phase, after the rehearsal rather than inside it: the rehearsal is
    a local cohort that fetches nothing, and this reads the public channel.
    A release lane skips it, because there it is not a replay -- it is the lane.
    """
    if qualification.pulled:
        return after[-1]
    phase = plan.phase(PHASE)
    previous = after
    for fetched in fetch_before(config):
        previous = (phase.add(fetched, after=previous),)
    return phase.add(transition_step(config), after=previous)


def transition_step(config: GateConfig) -> Step:
    return step(
        "installed",
        Call(
            "install the public release and let it update itself to the candidate",
            lambda context: TransitionGate(
                context.runner, source_commit=str(source_commit_for_checkout(config.root))
            ).run(),
            justification=CallJustification(
                kind=OpaqueKind.DOMAIN_TRANSACTION,
                reason="the installed update transition, as one container transaction",
                effects=machine_effects(
                    Effect.PROCESS, Effect.FILESYSTEM, Effect.NETWORK, Effect.HOST_STATE
                ),
            ),
        ),
        contends=(config.exclusive("docker_daemon"),),
        kind=Kind.E2E,
        needs=frozenset({Needs.DOCKER, Needs.DISK, Needs.VM}),
        speed=Speed.SLOW,
    )
