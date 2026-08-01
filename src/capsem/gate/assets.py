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

import shutil
import tempfile
from contextlib import suppress
from pathlib import Path

from . import config as gate_config
from . import host, pidfiles
from .actions import Call
from .assetlanes import AssetLanes, Profile, discover_profiles
from .command import GateCommand
from .errors import GateError
from .execution import step
from .plan import Plan
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

    # -- preflight ---------------------------------------------------------

    def _cross_architecture(self) -> gate_config.Arch:
        """The architecture this host is not."""
        others = [
            arch
            for arch in self._config.architectures.values()
            if arch.name != self.host_arch.name
        ]
        if len(others) != 1:
            raise GateError(
                "the asset gate expects exactly one non-host architecture, got "
                f"{[arch.name for arch in others]}"
            )
        return others[0]

    def _require_cross_execution(self, other: gate_config.Arch) -> None:
        """Prove Docker can run the other architecture before building for it."""
        platform = f"{self._assets.cross_platform_prefix}{other.dpkg}"
        self._runner.step(f"Ironbank {other.name} container execution preflight")
        probe = [
            "docker", "run", "--rm", "--platform", platform,
            self._assets.cross_platform_probe_image,
            self._assets.cross_platform_probe_command,
        ]
        if self._runner.succeeds(probe):
            return

        remedy = (
            "Colima Rosetta may be configured but stale; run 'colima restart' and retry."
            if host.on_macos()
            else "Install/register binfmt QEMU support and retry."
        )
        raise GateError(f"Docker cannot execute {platform} containers.\n{remedy}")

    # -- per-profile assembly ----------------------------------------------

    def _profile_root(self, profile: Profile) -> Path:
        return self.test_root / profile.name

    def _merge_lanes(self, profile: Profile, lanes: AssetLanes) -> Path:
        assets = self._profile_root(profile) / self._assets.merged_assets_dir
        assets.mkdir(parents=True, exist_ok=True)
        for arch in self._config.architectures.values():
            built = lanes.lane_assets(profile, arch) / arch.name
            shutil.copytree(built, assets / arch.name, dirs_exist_ok=True)
        return assets

    def _admin(self, *args: str) -> list[str]:
        return [*self._assets.admin_command, *args]

    def _publish(self, assets: Path) -> str:
        """Generate, alias, and check the merged manifest; return its file URI."""
        self._runner.run(self._admin("manifest", "generate", str(assets)))

        current = assets / self._assets.current_link
        with suppress(FileNotFoundError):
            current.unlink()
        current.symlink_to(self.host_arch.name)
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
                    "profile", "materialize",
                    "--profile", str(runtime.manifest),
                    "--config-root", self._assets.merged_config_dir,
                    "--manifest", manifest_uri,
                    "--assets-dir", str(assets),
                    "--output-root", str(output),
                    "--arch", self.host_arch.name,
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
        home.mkdir(parents=True, exist_ok=True)
        # AF_UNIX paths must stay under macOS SUN_LEN once the gateway appends
        # `instances/<uuid>-ws.sock` -- 54 characters -- and test_root is
        # already too long. The template names the *directory*, not just a
        # prefix: `mkdtemp` without `dir=` uses $TMPDIR, which on macOS is
        # `/var/folders/<11>/<24>/T/` and blows the 104-byte limit on its own.
        template = Path(self._assets.run_dir_template)
        template.parent.mkdir(parents=True, exist_ok=True)
        run_dir = Path(
            tempfile.mkdtemp(prefix=template.name.split(".")[0] + ".", dir=template.parent)
        )
        marker = (
            f"CAPSEM_ASSET_{profile.name.replace('-', '_')}_{self.host_arch.name}_SHELL_OK"
        )

        try:
            self._runner.script(
                self._assets.shell_proof_script,
                "--capsem", self._config.path(self._assets.capsem_binary),
                "--marker", marker,
                "--session-name", f"asset-{profile.name}-{self.host_arch.name}",
                "--profile", profile.name,
                "--timeout", self._assets.shell_proof_timeout_seconds,
                env={
                    "CAPSEM_HOME": str(home),
                    "CAPSEM_RUN_DIR": str(run_dir),
                    "CAPSEM_ASSETS_DIR": str(assets),
                    "CAPSEM_PROFILES_DIR": str(
                        config_root / self._assets.materialized_profiles_dir
                    ),
                },
            )
        except GateError:
            # Stop the service first: it SIGTERMs every VM process, flushing
            # process.log and serial.log, which are what a boot failure is
            # argued from.
            pidfiles.stop_gate_service(run_dir, self._config.pidfiles)
            self._preserve_evidence(profile, run_dir)
            raise
        finally:
            pidfiles.stop_gate_service(run_dir, self._config.pidfiles)
            shutil.rmtree(run_dir, ignore_errors=True)

    def _preserve_evidence(self, profile: Profile, run_dir: Path) -> None:
        """Copy the host-side diagnostics out before the run directory goes."""
        destination = self._profile_root(profile) / self._assets.failure_evidence_dir
        shutil.rmtree(destination, ignore_errors=True)
        destination.mkdir(parents=True, exist_ok=True)

        for source in run_dir.rglob("*"):
            relative = source.relative_to(run_dir)
            if set(relative.parts) & set(self._assets.evidence_prune_dirs):
                continue
            if not source.is_file() or source.suffix not in self._assets.evidence_suffixes:
                continue
            target = destination / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            with suppress(OSError):
                shutil.copy(source, target)

        self._runner.note(f"Preserved asset-gate failure evidence in {destination}")

    # -- the run -----------------------------------------------------------

    def run(self) -> None:
        other = self._cross_architecture()
        self._require_cross_execution(other)
        # Resolve the asset rail from the checked-in storage policy: the
        # dual-architecture BuildKit cohort survives unless the daemon falls
        # below its declared reserve.
        self._storage.ensure_space("assets")

        shutil.rmtree(self.test_root, ignore_errors=True)
        self.test_root.mkdir(parents=True)

        profiles = discover_profiles(self._config)
        lanes = AssetLanes(self._runner, self._config, profiles)
        try:
            lanes.run(tuple(self._config.architectures.values()))
        finally:
            self._runner.run(
                ["bash", str(self._config.path(self._assets.container_cleanup_script)),
                 str(self.test_root)],
                check=False,
            )

        for profile in profiles:
            assets = self._merge_lanes(profile, lanes)
            manifest_uri = self._publish(assets)
            config_root = self._materialize(profile, assets, manifest_uri)
            self._prove(profile, assets, config_root)

        self._runner.note(
            "Ironbank VM asset build and boot gate passed for every profile "
            "and architecture."
        )


def assets_step(config):
    """Build every profile's VM assets and boot each one.

    Claims the Docker daemon, which it always did in fact -- it drives the
    image builds -- but only implicitly, through the `assets` command's own
    machine lock. Composed into a larger plan that lock is gone, so the
    contention has to be declared where the graph can see it.
    """
    return step(
        "assets",
        Call(
            "build and boot every profile's VM assets",
            lambda ctx: AssetGate(ctx.runner).run(),
        ),
        contends=(config.exclusive("docker_daemon"),),
    )


class AssetsCommand(
    GateCommand, name="assets", help="build every profile's VM assets and boot each one"
):
    exclusive = True

    def plan(self) -> Plan:
        plan = Plan(self.name)
        plan.add(assets_step(self._config))
        return plan
