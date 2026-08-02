"""The package rail: one Linux release cohort, built in the builder container.

Native cross-compilation, no QEMU. The image runs on the host architecture and
targets the other through `--target` plus multiarch system libraries, with
named volumes carrying the cargo registry, the rustup toolchain, and the build
directory between runs. `CARGO_TARGET_DIR=/cargo-target` inside the container
keeps all of that off the host's `target/`.

Keep in sync with `.github/workflows/release.yaml`'s `build-app-linux` job,
which does the same work on a bare Ubuntu runner: CI takes its Tauri signing
keys from secrets where this takes them from `private/tauri/`.

The build itself is `scripts/build-linux-package.sh`. It used to be the
argument of a `bash -c` inside a `docker run` inside a recipe, escaped twice
over and written as one logical line; as a file it is syntax-checked with the
rest of the shell in the repository.
"""

from __future__ import annotations

import os
import shutil
from pathlib import Path

from . import config as gate_config
from . import debproof, host, hostimage
from .actions import Call
from .command import GateCommand
from .config import Arch
from .errors import GateError
from .execution import step
from .fileactions import remove
from .packageinputs import package_environment, pinned_toolchain, resolve_channel
from .packagesigning import signing_key
from .plan import Plan
from .proc import Runner
from .storage import Storage


class PackageRail:
    """Builds, records, and optionally proves one architecture's package."""

    def __init__(
        self,
        runner: Runner,
        target: Arch,
        *,
        manifest_url: str | None = None,
        channel: str | None = None,
        require_proof: bool = False,
    ) -> None:
        self._runner = runner
        self._config = gate_config.for_root(runner.root)
        self._package = self._config.package
        self._storage = Storage(runner)
        self.root = runner.root
        self.target = target
        self.manifest_url = manifest_url or self._package.default_manifest_url
        self.channel = resolve_channel(channel or self._package.default_channel, self._config)
        self._require_proof = require_proof

    @property
    def _dist(self) -> Path:
        return self.root / self._package.dist_dir

    @property
    def _record(self) -> Path:
        return self._dist / f".cross-compile-{self.target.name}-deb"

    def run(self) -> Path:
        self._prepare_builder()
        self._sync_assets_for_tauri()
        self._build()
        package = self._recorded_package()
        self._prove(package)

        self._runner.step("Artifacts")
        self._runner.run(["ls", "-lh", str(self._dist)])
        self._storage.gc()
        return package

    # -- before the build --------------------------------------------------

    def _prepare_builder(self) -> None:
        self._storage.release("completed-docker-rails")
        # `deferred-install-target` is not released here: the package phase
        # owns it as a step, between the two architectures, which is the only
        # arrangement that can be ordered against the second build.
        # One named policy owns this rail: the Rust base image and the BuildKit
        # cohort stay warm across candidates, and a capacity failure reports an
        # explicit disk recommendation instead of silently building cold.
        # Once the builder image exists, because that image is itself part of
        # what fills this rail. The second check is in `_build`, immediately
        # before the package build spends it -- these were adjacent lines, so
        # both measured the same moment and the pair proved nothing.
        self._storage.ensure_space("package")
        if host.on_macos():
            # Colima's VM clock drifts, and apt rejects a repository signed in
            # what it believes is the future.
            self._runner.run(["python3", self._package.clock_script])

    def _sync_assets_for_tauri(self) -> None:
        """`assets/current` is what the bundler embeds; point it at this target.

        Through the primitives rather than `rm -rf` and `cp -r`: a raw `rm`
        with a path built in Python is the one shape the reclaimer guards
        exist to prevent, and neither showed up in a dry run.
        """
        settings = self._config.imagebuild
        current = self.root / settings.output / self._package.current_assets
        built = self.root / settings.output / self.target.name
        remove(current)
        if built.is_dir():
            shutil.copytree(built, current)

    # -- the build ---------------------------------------------------------

    def _build(self) -> None:
        self._runner.step(
            f"Building Linux deb ({self.target.name} via docker, target={self.target.rust_target})"
        )
        # Again, here: the builder image and the asset sync have landed since
        # the first reservation, and this is the point where being wrong about
        # capacity costs an hour of compilation.
        self._storage.ensure_space("package")
        self._dist.mkdir(exist_ok=True)
        self._record.unlink(missing_ok=True)

        environment = package_environment(
            self._config,
            self.target,
            toolchain=pinned_toolchain(self.root),
            manifest_url=self.manifest_url,
            signing=signing_key(self.root, self._config),
        )
        argv = ["docker", "run", "--rm"]
        for name, value in environment.items():
            argv += ["-e", f"{name}={value}"]
        mount = self._config.install.mount
        argv += ["-v", f"{self.root}:{mount}"]
        for volume in self._package.volumes:
            argv += ["-v", f"{volume.source}:{volume.target}"]
        argv += [
            "-v",
            f"{self._package.target_volume_for(self.target.name)}"
            f":{self._package.cargo_target_mount}",
            "-w",
            mount,
            self._package.builder_image,
            "bash",
            f"{mount}/{self._package.build_script}",
        ]
        self._runner.run(argv)

    def _recorded_package(self) -> Path:
        """The exact package this run produced, not whatever `dist/` holds.

        The builder writes the basename it just created. Globbing `dist/`
        instead would happily prove and publish a package left by an earlier
        build of a different commit.
        """
        if not self._record.is_file() or not self._record.read_text().strip():
            raise GateError("builder did not record the exact Debian package")
        name = self._record.read_text(encoding="utf-8").strip()
        self._record.unlink()

        if not name.endswith(self._package.package_suffix):
            raise GateError(f"invalid Debian package record: {name}")
        if name != Path(name).name:
            raise GateError(f"Debian package record escaped {self._package.dist_dir}/: {name}")

        package = self._dist / name
        if not package.is_file():
            raise GateError(f"recorded Debian package is missing: {package}")
        return package

    # -- after the build ---------------------------------------------------

    def _prove(self, package: Path) -> None:
        native = self._config.host_arch()
        kvm_ready = all(host.device_available(device) for device in self._config.install.vm_devices)
        decision = self._runner.capture(
            [
                "bash",
                str(self.root / self._package.proof_selector),
                host.system(),
                native.name,
                self.target.name,
                "1" if kvm_ready else "0",
                "1" if self._require_proof else "0",
            ]
        )
        if decision != "prove":
            self._runner.note(
                "Skipping exact Debian package proof for non-host or optional "
                f"target ({host.system()}/{native.name} -> {self.target.name})."
            )
            return

        self._runner.step("Proving exact Debian package in systemd + KVM")
        # Called, not launched. The three `CAPSEM_PROOF_*` variables existed
        # only to carry these arguments across a process boundary that no
        # longer exists -- and `DebProof` always took them as arguments.
        debproof.DebProof(
            self._runner,
            package=package,
            manifest_url=self.manifest_url,
            channel=self.channel,
        ).run()


def fragment(plan: Plan, config, target, *, after: tuple = ()):
    """One architecture's package, after the builder image it needs.

    The builder is `shared`, so composing several architectures into one plan
    builds it once and hangs every lane off it.

    `after` reaches the package step and deliberately not the image. The
    glow-up lane chains architectures so the second build waits for the first
    to release its disk; passing that down made the shared image depend on a
    package that depends on the image, which is a cycle -- and one that appears
    only once two lanes share a plan. Groundwork has no ordering of its own.
    """
    built = hostimage.fragment(plan, config)
    return plan.add(
        step(
            f"package.{target.name}",
            Call(
                f"build the Linux release package for {target.name}",
                lambda ctx: _build(ctx, target),
            ),
            contends=(config.exclusive("docker_daemon"),),
        ),
        after=(built, *after),
    )


class CrossCompileCommand(
    GateCommand,
    name="cross-compile",
    help="build the Linux release package for one architecture",
):
    exclusive = True

    @classmethod
    def add_arguments(cls, parser) -> None:
        parser.add_argument("arch", nargs="?", help="arm64 or x86_64; defaults to the host")

    def plan(self) -> Plan:
        config = self._config
        target = config.arch(self._args.arch) if self._args.arch else config.host_arch()
        plan = Plan(self.name)
        fragment(plan, config, target)
        return plan


def _build(context, target) -> None:
    settings = context.config.package
    PackageRail(
        context.runner,
        target,
        manifest_url=os.environ.get(settings.manifest_variable),
        channel=os.environ.get(settings.channel_variable),
        require_proof=os.environ.get(settings.require_proof_variable, "0") == "1",
    ).run()
