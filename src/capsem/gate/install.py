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
from pathlib import Path

from . import config as gate_config
from . import hostimage, installimage
from .actions import Call, Why
from .command import GateCommand
from .docker import Docker, container_path
from .errors import GateError
from .execution import step
from .fileactions import remove
from .installcontainer import InstallContainer
from .installproof import InstallProof
from .plan import Plan
from .proc import Runner
from .releasegraph import ReleaseGraph
from .storage import Storage
from .versions import workspace_version


class InstallGate:
    """One run of the native install and glow-up proof."""

    def __init__(
        self,
        runner: Runner,
        *,
        profile_inputs: str | None = None,
        macos_glowup_report: str | None = None,
    ) -> None:
        self._runner = runner
        self._config = gate_config.for_root(runner.root)
        self._settings = self._config.install
        self._layout = self._settings.layout
        self._storage = Storage(runner)
        self._container = InstallContainer(runner)
        self._proof = InstallProof(runner, self._config)
        self._graph = ReleaseGraph(Docker(runner), self._config)
        self._profile_inputs = profile_inputs or None
        self._macos_report = macos_glowup_report or None
        self.root = runner.root
        self.version = workspace_version(runner.root)
        self.arch = self._config.host_arch()

    @property
    def package(self) -> Path:
        return self.root / "dist" / f"Capsem_{self.version}_{self.arch.dpkg}.deb"

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
        options = self._container.runtime_options()

        # The rails this used to release -- `deferred-install-target` and both
        # `completed-package-*` -- are steps in the phase that composes this,
        # hanging off the builds that fill them. Released from here they had no
        # edges, so nothing could order them against another lane's work.
        # The verified derived image pins roughly 6 GiB until this proof
        # finishes. Reserve that budget before materialising it.
        self._storage.ensure_space("install-preflight")
        installimage.prepare(self._runner)

        # A failed site overlay can leave write-only partial HTML on a macOS
        # bind mount. The host owns this generated tree, so clear it before the
        # container exists; profile artifacts are regenerated from the manifest.
        remove(self._config.path(self._layout.channel))

        try:
            self._container.require_rosetta()
            self._storage.ensure_space("install")
            self._container.start(options=options)
            self._prove(package)
        finally:
            # Ordered: clear the handoff before the container goes, or the next
            # install in this checkout inherits a request pointing at a graph
            # that no longer exists.
            self._graph.clear_handoff()
            self._container.return_paths()
            self._container.stop()
            self._storage.release("after-install", best_effort=True)
            self._storage.gc(rail="install", best_effort=True)

        self._container.verify_rosetta_survived()

    def _prove(self, package: str) -> None:
        packaged = self._proof.packaged_version(package)
        if packaged != self.version:
            raise GateError(
                f"{self.package.name} declares version {packaged}, but this "
                f"checkout is {self.version}"
            )

        # Before dpkg, and from the package under test: this is what breaks the
        # circle between the graph and the binary that authors it.
        admin = self._graph.extract_admin(package)

        self._runner.note("Staging real profile assets for installed VM proofs...")
        authoritative_graph = self._stage()

        self._runner.note("Authoring exact candidate manifest for the installed package...")
        self._graph.record_binary(
            admin,
            package=package,
            version=self.version,
            assets_manifest=f"{self._layout.assets}/{self._settings.manifest_name}",
            candidate_base=f"{self._settings.mount}/{self._layout.packages}",
        )

        if authoritative_graph:
            self._publish_local_channel(admin)

        # The manifest handed over, read back from what the postinst recorded.
        self._proof.install(package, expected=self.version, manifest=self._graph.handed_off)
        self._graph.clear_handoff()

        # The Linux CI container chowns the bind-mounted checkout to uid 1000 so
        # its non-root build can write there. Hand the host-owned storage ledger
        # back before invoking the host controller; cleanup restores the rest.
        self._container.hand_back(f"{self._settings.mount}/{self._settings.storage_ledger}")
        # Package and image assembly can consume the reserve measured at start.
        # The runtime-only tail needs far less than compilation, but keeps a
        # cushion so ENOSPC fails here with diagnostics rather than deep inside
        # a fixture after hours of otherwise-green release work.
        self._storage.ensure_space("install")

        self._proof.run_install_suite()
        if not self._container.boots_a_guest:
            self._proof.validate_macos_glowup(
                self._macos_report,
                cargo_toml=self._config.path(self._config.versions.cargo_manifest),
            )
        self._proof.prove_glowup(package, boots_a_guest=self._container.boots_a_guest)

    def _stage(self) -> bool:
        """Stage assets, and report whether a local graph must be published.

        A release lane's profile inputs are already manifest-resolved and
        verified, and carry no authoritative graph for this checkout to hand
        over. A local gate publishes one from its own freshly built assets.
        """
        if self._profile_inputs:
            self._proof.stage_verified_inputs(self._profile_inputs)
            return False
        self._proof.stage_local_assets()
        return True

    def _publish_local_channel(self, admin: str) -> None:
        self._runner.note("Generating authoritative local release graph...")
        manifest = self._graph.build_channel(
            admin,
            assets_dir=self._layout.assets,
            profiles_dir=f"{self._layout.config}/{self._config.assets.materialized_profiles_dir}",
            channel=self._settings.channel,
            manifest_version=self._settings.manifest_version,
            out_dir=self._layout.channel,
        )
        self._graph.build_site(dist=self._layout.channel)
        self._graph.check_channel(
            admin,
            channel=self._settings.channel,
            dist=self._layout.channel,
            manifest=manifest,
        )
        # Last thing before dpkg, and only once the graph it names has been
        # built and checked.
        self._graph.hand_off(manifest)


def install_step(config):
    """Install the exact package and prove the installed product.

    Claims the Docker daemon explicitly. It always drove a privileged container
    and a storage rail; composed into a larger plan there is no per-command
    machine lock left to make that true by accident.
    """
    return step(
        "install",
        Call(
            "install the exact package and prove the installed product", _install, why=Why.DYNAMIC
        ),
        contends=(config.exclusive("docker_daemon"),),
    )


class InstallCommand(
    GateCommand,
    name="install",
    help="install the exact release package and prove it works",
):
    exclusive = True

    def plan(self) -> Plan:
        plan = Plan(self.name)
        # `docker/Dockerfile.install-test` is `FROM capsem-host-builder:latest`
        # and this lane rebuilds that derived image itself, so the base is a
        # prerequisite it owns rather than one an earlier phase happens to
        # leave behind. `shared`, so composing this into the complete gate
        # makes it a dependant of the one build rather than a second one.
        base = hostimage.fragment(plan, self._config)
        plan.add(install_step(self._config), after=(base,))
        return plan


def _install(context) -> None:
    config = context.config
    InstallGate(
        context.runner,
        profile_inputs=os.environ.get(config.install.profile_inputs_variable),
        # Already declared; this spelled it a second time, so the two could
        # drift and the report would simply stop arriving.
        macos_glowup_report=os.environ.get(config.modules.macos_report_variable),
    ).run()
