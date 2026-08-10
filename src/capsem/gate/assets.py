"""Build every profile's VM assets, then boot each one and prove it works.

Building is `assetlanes`; this is what happens either side of it. Before: check
that Docker can execute the *other* architecture at all, because discovering
otherwise an hour in wastes the whole matrix. After, per profile: merge both
lanes into one asset tree, generate and check its manifest, materialize the
runtime profiles against it, and boot a real VM through
`prove-installed-shell.py`.

Two details are load-bearing and neither is obvious.

`current` is a symlink the image builders repoint at whichever architecture
they built last, so the merged manifest generator leaves it pointing wherever
the final lane happened to finish. The host-architecture VM proof that follows
needs it aimed at *this* machine, so it is restored and then verified.

The installed runtime resolves content-addressed filenames, while build output
uses canonical logical names. Without the same zero-copy hash aliases that
package and dev preparation materialize, startup falls through to a remote
fetch for a local-only asset version -- and the gate silently stops being
hermetic.
"""

from __future__ import annotations

import os
from pathlib import Path

from . import assetevidence, crossexec, imagebases, pidfiles
from . import config as gate_config
from .assetlanes import AssetLanes, Profile, discover_profiles
from .errors import GateError
from .fileactions import (
    copy_tree,
    discard,
    link,
    make_dir,
    merge_tree,
    remove,
    scratch_dir,
)
from .proc import Runner
from .storage import Storage


class AssetGate:
    """One run of the VM asset build and boot proof, for every profile."""

    def __init__(self, runner: Runner, *, sleep=None) -> None:
        self._runner = runner
        self._config = gate_config.for_root(runner.root)
        self._assets = self._config.assets
        self._storage = Storage(runner)
        self.root = self._config.root
        self.test_root = self._config.path(self._assets.test_root)
        self.host_arch = self._config.host_arch()

    @property
    def build_config(self):
        """The exact image inputs copied into every materialized profile."""
        return imagebases.build_config(self._config)

    # -- preflight ---------------------------------------------------------

    def _profile_root(self, profile: Profile) -> Path:
        return self.test_root / profile.name

    def _merge_lanes(self, profile: Profile, lanes: AssetLanes) -> Path:
        assets = self._profile_root(profile) / self._assets.merged_assets_dir
        make_dir(assets)
        for arch in self._config.architectures.values():
            built = lanes.lane_assets(profile, arch) / arch.name
            merge_tree(built, assets / arch.name)
        return assets

    def _admin(self, *args: str) -> list[str]:
        return [*self._assets.admin_command, *args]

    def _publish(self, assets: Path) -> str:
        """Generate, alias, and check the merged manifest; return its file URI."""
        self._runner.run(self._admin("manifest", "generate", str(assets)))

        current = assets / self._assets.current_link
        link(current, self.host_arch.name)
        if current.readlink().name != self.host_arch.name:
            raise GateError(
                f"{current} points at {current.readlink()}, not the host "
                f"architecture {self.host_arch.name}"
            )

        self._runner.script(self._assets.hash_assets_script, assets)
        manifest = assets / self._config.install.manifest_name
        self._runner.run(self._admin("manifest", "check", str(manifest)))
        return manifest.resolve().as_uri()

    def _materialize(self, profile: Profile, assets: Path, manifest_uri: str) -> Path:
        """Materialize every runtime profile against this profile's assets."""
        output = self._profile_root(profile) / self._assets.merged_config_dir
        for runtime in discover_profiles(self._config):
            self._runner.run(
                self._admin(
                    "profile",
                    "materialize",
                    "--profile",
                    str(runtime.manifest),
                    "--config-root",
                    self._assets.merged_config_dir,
                    "--manifest",
                    manifest_uri,
                    "--assets-dir",
                    str(assets),
                    "--output-root",
                    str(output),
                    "--arch",
                    self.host_arch.name,
                )
            )
        return output

    # -- the boot proof ----------------------------------------------------

    def _prove(self, profile: Profile, assets: Path, config_root: Path) -> None:
        home = (
            self._profile_root(profile)
            / self._assets.profile_home_dir
            / self._config.install.capsem_home
        )
        make_dir(home)
        # AF_UNIX paths must stay under macOS SUN_LEN once the gateway appends
        # `instances/<uuid>-ws.sock` -- 54 characters -- and test_root is
        # already too long. The template names the *directory*, not just a
        # prefix: `mkdtemp` without `dir=` uses $TMPDIR, which on macOS is
        # `/var/folders/<11>/<24>/T/` and blows the 104-byte limit on its own.
        template = Path(self._assets.run_dir_template)
        run_dir = scratch_dir(template.name.split(".")[0] + ".", template.parent)
        marker = f"CAPSEM_ASSET_{profile.name.replace('-', '_')}_{self.host_arch.name}_SHELL_OK"

        names = self._config.environment
        try:
            self._runner.script(
                self._assets.shell_proof_script,
                "--capsem",
                self._config.path(self._assets.capsem_binary),
                "--marker",
                marker,
                "--session-name",
                f"asset-{profile.name}-{self.host_arch.name}",
                "--profile",
                profile.name,
                "--timeout",
                self._assets.shell_proof_timeout_seconds,
                env={
                    **names.capsem(home=home, run_dir=run_dir),
                    **names.content(
                        assets=assets,
                        profiles=config_root / self._assets.materialized_profiles_dir,
                    ),
                },
            )
        except GateError:
            # Stop the service first: it SIGTERMs every VM process, flushing
            # process.log and serial.log, which are what a boot failure is
            # argued from.
            pidfiles.stop_gate_service(run_dir, self._config.pidfiles)
            assetevidence.preserve(
                self._runner,
                self._config,
                destination=self._profile_root(profile) / self._assets.failure_evidence_dir,
                run_dir=run_dir,
            )
            raise
        finally:
            pidfiles.stop_gate_service(run_dir, self._config.pidfiles)
            discard(run_dir)

    def _select_base(self, profiles: list[Profile]) -> None:
        """Make later local phases consume the base profile IronBank proved.

        The per-profile trees stay private so functional can select each one.
        The canonical asset root is only a relative selector while this private
        checkout is alive; prefix export materializes it back into the caller.
        Generated profile configuration is small and copied, so it never
        becomes a dangling link when the private asset tree is reclaimed.
        """
        base_name = self._config.suites.pytest.base_profile
        base = next((profile for profile in profiles if profile.name == base_name), None)
        if base is None:
            raise GateError(f"base profile {base_name!r} was not built by the asset gate")

        profile_root = self._profile_root(base)
        assets = profile_root / self._assets.merged_assets_dir
        config_root = profile_root / self._assets.merged_config_dir
        profiles_dir = config_root / self._assets.materialized_profiles_dir
        manifest = assets / self._config.install.manifest_name
        config_manifest = config_root / self._config.suites.pytest.test_manifest
        if not manifest.is_file():
            raise GateError(f"verified base asset manifest is missing: {manifest}")
        if not profiles_dir.is_dir():
            raise GateError(f"verified base profile catalog is missing: {profiles_dir}")
        if not config_manifest.is_file():
            raise GateError(f"verified base config manifest is missing: {config_manifest}")
        if config_manifest.read_bytes() != manifest.read_bytes():
            raise GateError(
                f"verified base config manifest {config_manifest} does not match {manifest}"
            )
        for arch in self._config.architectures:
            if not (assets / arch).is_dir():
                raise GateError(f"verified base assets are missing architecture {arch}: {assets}")

        canonical_assets = self._config.path(self._config.functional.assets_dir)
        relative = os.path.relpath(assets, canonical_assets.parent)
        remove(canonical_assets)
        link(canonical_assets, relative)
        if canonical_assets.resolve() != assets.resolve():
            raise GateError(
                f"canonical assets selected {canonical_assets.resolve()}, not {assets.resolve()}"
            )

        canonical_config = self._config.path(self._config.functional.config_root)
        copy_tree(config_root, canonical_config)
        self._runner.note(
            f"Selected Ironbank profile {base_name} for build-chain, packaging, and glow-up."
        )

    def preflight(self) -> None:
        """Refuse a build the daemon cannot finish, and clear the tree."""
        crossexec.require(self._runner, self._config, self.host_arch)
        # Resolve the asset rail from the checked-in storage policy: the
        # dual-architecture BuildKit cohort survives unless the daemon falls
        # below its declared reserve.
        self._storage.ensure_space("assets")
        discard(self.test_root)
        make_dir(self.test_root)

    def prefetch(self) -> None:
        """Materialize exact bases through the Docker daemon's fetch edge."""
        imagebases.prefetch(self._runner, self._config)

    def lane(self, arch_name: str) -> None:
        """One architecture's builds, across every profile."""
        AssetLanes(self._runner, self._config, discover_profiles(self._config)).build(
            self._config.arch(arch_name)
        )

    def sweep(self) -> None:
        """Containers the lanes may have left behind."""
        self._runner.run(
            [
                "bash",
                str(self._config.path(self._assets.container_cleanup_script)),
                str(self.test_root),
            ],
            check=False,
        )

    def assemble(self) -> None:
        """Merge each profile's lanes, publish, materialise, and boot it."""
        profiles = discover_profiles(self._config)
        lanes = AssetLanes(self._runner, self._config, profiles)
        for profile in profiles:
            assets = self._merge_lanes(profile, lanes)
            manifest_uri = self._publish(assets)
            config_root = self._materialize(profile, assets, manifest_uri)
            self._prove(profile, assets, config_root)

        self._select_base(profiles)

        self._runner.note(
            "Ironbank VM asset build and boot gate passed for every profile and architecture."
        )
