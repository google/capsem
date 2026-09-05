"""Install the exact package about to be published, and prove it works.

Deliberately the *release-mode* package the package rail already produced.
Rebuilding a debug package here would both waste that work and prove bytes
that can never be published.

This file is the order. Every step below depends on the one before it, and the
one the shell got wrong was the handoff: `capsem-admin` authors the release
graph and ships *inside* the package under test, so the shell installed first
and authored afterwards -- leaving the postinst with no manifest to read. It
does not fail in that case. It silently hydrates from the URL baked into the
package, so the whole-world local proof was reaching `release.capsem.org`, and
broke when those public artifacts were retired. See `releasegraph`.
"""

from __future__ import annotations

import os
from collections.abc import Mapping
from pathlib import Path

from . import cachelayout, installplan, platformproof, runtimeprepare
from . import config as gate_config
from .actions import Call
from .cachecontrol import CacheControl
from .command import GateCommand
from .content import InstallContent, LocalInstallContent, ProfileContent, SelectedInstallContent
from .docker import Docker
from .dockermount import container_path
from .errors import GateError
from .execution import Kind, Needs, Speed, step
from .fileactions import make_dir, remove
from .installcontainer import InstallContainer
from .installproof import InstallProof
from .opacity import CallJustification, Effect, OpaqueKind, machine_effects
from .plan import Plan
from .proc import Runner
from .releasegraph import ReleaseGraph
from .sourcecommit import SourceCommit, source_commit_for_checkout
from .versions import workspace_version


class InstallGate:
    """One run of the native install and glow-up proof."""

    def __init__(
        self,
        runner: Runner,
        *,
        content: InstallContent | None = None,
        macos_glowup_report: str | None = None,
        source_commit: SourceCommit,
    ) -> None:
        self._runner = runner
        self._config = gate_config.for_root(runner.root)
        self._settings = self._config.install
        self._layout = self._settings.layout
        self._cache = CacheControl(runner)
        self._content = content
        self._container = InstallContainer(runner, content=content)
        self._proof = InstallProof(
            runner,
            self._config,
            source_commit=source_commit,
        )
        self._graph = ReleaseGraph(Docker(runner), self._config, source_commit=source_commit)
        self._macos_report = macos_glowup_report or None
        self.root = runner.root
        self.version = workspace_version(runner.root)
        self.arch = self._config.host_arch()

    @property
    def package(self) -> Path:
        return self._config.path(self._config.outputs.packages) / (
            f"Capsem_{self.version}_{self.arch.dpkg}.deb"
        )

    def _require_package(self) -> str:
        if not self.package.is_file() or self.package.stat().st_size == 0:
            raise GateError(
                f"missing exact release-mode Debian package: {self.package}\n"
                f"Run the package rail first: just _cross-compile {self.arch.name}"
            )
        return container_path(self.root, self.package, mount=self._settings.mount)

    # -- the run -----------------------------------------------------------

    def run(self) -> None:
        package = self._require_package()
        platformproof.prove(self._runner, self._config, self.package)
        self._require_content()
        options = self._container.runtime_options()

        # The rails this used to release -- `deferred-install-target` and both
        # `completed-package-*` -- are steps in the phase that composes this,
        # hanging off the builds that fill them. Released from here they had no
        # edges, so nothing could order them against another lane's work.
        # A failed site overlay can leave write-only partial HTML on a macOS
        # bind mount. The host owns this generated tree, so clear it before the
        # container exists; profile artifacts are regenerated from the manifest.
        remove(self._config.path(self._layout.channel))
        evidence = self._config.path(self._layout.glowup_evidence)
        remove(evidence)
        make_dir(evidence)

        try:
            self._container.require_rosetta()
            self._cache.enforce("docker", "install")
            self._container.start(options=options)
            self._prove(package)
        except GateError:
            self._container.capture_storage_failure()
            raise
        finally:
            # Ordered: clear the handoff before the container goes, or the next
            # install in this checkout inherits a request pointing at a graph
            # that no longer exists.
            self._graph.clear_handoff()
            self._container.return_paths()
            self._container.stop()
            self._cache.prune(best_effort=True)

        self._container.verify_rosetta_survived()

    def _require_content(self) -> None:
        if self._content is None:
            raise GateError(
                "install proof requires one selected profile content bundle; "
                "pass --selected-content-root for manifest-selected content"
            )
        arches = None
        if isinstance(self._content, SelectedInstallContent):
            arches = (self.arch,)
            self._content.require_complete(self._config, arches=arches)
            self._runner.script(
                self._config.modules.verify_inputs_script,
                "--input-dir",
                self._content.inputs(self._config),
            )
            return
        self._content.content.require_complete(self._config, arches=arches)

    def _prove(self, package: str) -> None:
        packaged = self._proof.packaged_version(package)
        if packaged != self.version:
            raise GateError(
                f"{self.package.name} declares version {packaged}, but this "
                f"checkout is {self.version}"
            )

        self._runner.note("Staging real profile assets for installed VM proofs...")
        self._stage()

        # Before dpkg, and from the package under test: this breaks the circle
        # between the selected profile graph and the exact binary being
        # qualified. Even release-selected profile content needs this graph;
        # handing its raw projection to postinst would leave the package URL
        # public and make network-none fail for the wrong reason.
        self._runner.note("Authoring exact candidate manifest for the installed package...")
        self._graph.author_exact_package(
            package=package,
            version=self.version,
            assets_manifest=f"{self._layout.assets}/{self._settings.manifest_name}",
            candidate_base=f"{self._settings.mount}/{self._layout.packages}",
            assets_dir=self._layout.assets,
            profiles_dir=f"{self._layout.config}/{self._config.assets.materialized_profiles_dir}",
            channel=self._settings.channel,
            profile_revision_policy=self._settings.profile_revision_policy,
            manifest_version=self._settings.manifest_version,
            out_dir=self._layout.channel,
        )

        # The manifest handed over, read back from what the postinst recorded.
        self._proof.install(package, expected=self.version, manifest=self._graph.handed_off)
        self._container.verify_vm_device_access()
        self._graph.clear_handoff()

        # The Linux CI container chowns the bind-mounted checkout to uid 1000 so
        # its non-root build can write there. Hand the host-owned storage ledger
        # back before invoking the host controller; cleanup restores the rest.
        state = cachelayout.stage_relative_path(self._config, "state")
        self._container.hand_back(f"{self._settings.mount}/{state}")
        # Recheck owned usage after package installation, before VM fixtures.
        self._cache.enforce("docker", "install")

        self._proof.run_install_suite()
        if not self._container.boots_a_guest:
            self._proof.validate_macos_glowup(
                self._macos_report,
                cargo_toml=self._config.path(self._config.versions.cargo_manifest),
            )
        self._proof.prove_glowup(package, boots_a_guest=self._container.boots_a_guest)

    def _stage(self) -> None:
        """Stage the paired cohort and prepare its checked local graph root."""
        content = self._content
        if content is None:
            raise GateError("install content was not validated before staging")
        if isinstance(content, SelectedInstallContent):
            self._proof.verify_selected_inputs(content.inputs(self._config))
        self._proof.stage_content(content.content)
        self._proof.start_local_server()


def install_step(config, *, content: InstallContent):
    """Install the exact package and prove the installed product.

    Claims the Docker daemon explicitly. It always drove a privileged container
    and a storage rail; composed into a larger plan there is no per-command
    machine lock left to make that true by accident.
    """
    return step(
        "install",
        Call(
            "install the exact package and prove the installed product",
            lambda context: _install(context, content=content),
            justification=CallJustification(
                kind=OpaqueKind.DOMAIN_TRANSACTION,
                reason="install the exact package and prove the installed product, as one transaction",
                effects=machine_effects(
                    Effect.PROCESS,
                    Effect.FILESYSTEM,
                    Effect.NETWORK,
                    Effect.HOST_STATE,
                ),
            ),
        ),
        contends=(config.exclusive("docker_daemon"),),
        kind=Kind.E2E,
        needs=frozenset({Needs.DOCKER, Needs.DISK}),
        speed=Speed.SLOW,
    )


class InstallCommand(
    GateCommand,
    name="install",
    help="install the exact release package and prove it works",
):
    exclusive = True

    @classmethod
    def add_arguments(cls, parser) -> None:
        parser.add_argument(
            "--selected-content-root",
            help="paired assets/config root already selected from a release manifest",
        )

    def plan(self) -> Plan:
        plan = Plan(self.name)
        # Own the sealed image prerequisite. In the complete plan this shares
        # the static preflight step; standalone install builds it once here.
        image = installplan.fragment(plan, self._config)
        selected = getattr(self._args, "selected_content_root", None)
        prerequisites = (image,)
        if selected:
            root = Path(selected)
            root = root if root.is_absolute() else self._config.path(str(root))
            content: InstallContent = SelectedInstallContent(
                ProfileContent.isolated(self._config, root)
            )
        else:
            content = LocalInstallContent(ProfileContent.standalone(self._config))
            prepared = plan.add(runtimeprepare.materialize_config_step(self._config))
            prerequisites += (prepared,)
        plan.add(install_step(self._config, content=content), after=prerequisites)
        return plan


def macos_report(config, environ: Mapping[str, str] | None = None) -> str | None:
    """Where the native macOS glow-up proof left its report, if it ran.

    Two ways in, and it only ever had one. A release lane produces the report
    in another job and hands it over by variable; a local gate produces it in
    the `macos-package` step immediately before this one, which writes it at
    the configured path and exports nothing.

    So the variable was read, nothing set it, and every complete local gate on
    macOS failed at its very last step with "requires the native glow-up report
    from this module" -- while the report sat exactly where `[modules]` said it
    would. Returning `None` when neither exists keeps the refusal for the case
    it was written for: the proof genuinely did not run.
    """
    source = os.environ if environ is None else environ
    handed = (source.get(config.modules.macos_report_variable) or "").strip()
    if handed:
        return handed
    written = config.path(config.modules.macos_glowup_report)
    return str(written) if written.is_file() else None


def _install(context, *, content: InstallContent) -> None:
    config = context.config
    InstallGate(
        context.runner,
        content=content,
        macos_glowup_report=macos_report(config),
        source_commit=source_commit_for_checkout(config.root),
    ).run()
