"""One architecture's Linux release package, built in the builder container.

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

Split from `crosscompile`, which turns these methods into plan steps. The seam
is the one the module ceiling kept pointing at: what the rail *does* against a
machine, and how a graph *orders* it, change for different reasons.
"""

from __future__ import annotations

from pathlib import Path

from . import config as gate_config
from . import debproof, host
from .config import Arch
from .docker import Docker
from .dockermount import Mount
from .errors import GateError
from .fileactions import copy_tree, make_dir, remove
from .gitmetadata import docker_git_metadata_mount
from .packageinputs import package_environment, pinned_toolchain, resolve_channel
from .packagesigning import signing_key
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

    # -- the phases, each one a step ---------------------------------------
    #
    # Stateless between phases: `resolve` is a pure read of what the builder
    # recorded, so `prove` asks again rather than holding an object an earlier
    # step mutated. See `fragment` for why they are separate at all.

    def release_rails(self) -> None:
        self._storage.release("completed-docker-rails")
        # `deferred-install-target` is not released here: the package phase
        # owns it as a step, between the two architectures, which is the only
        # arrangement that can be ordered against the second build.
        # One named policy owns this rail: the Rust base image and the BuildKit
        # cohort stay warm across candidates, and a capacity failure reports an
        # explicit disk recommendation instead of silently building cold.

    def reserve(self) -> None:
        """Once the builder image exists, because that image fills this rail.

        The second reservation is in `build`, immediately before the package
        spends it. These were adjacent lines, so both measured the same moment
        and the pair proved nothing.
        """
        self._storage.ensure_space("package")

    def sync_clock(self) -> None:
        """Colima's VM clock drifts, and apt rejects a repository signed in
        what it believes is the future."""
        if host.on_macos():
            self._runner.run(["python3", self._package.clock_script])

    def sync_assets(self) -> None:
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
            copy_tree(built, current)

    # -- the build ---------------------------------------------------------

    def build(self) -> None:
        self._runner.step(
            f"Building Linux deb ({self.target.name} via docker, target={self.target.rust_target})"
        )
        # Again, here: the builder image and the asset sync have landed since
        # the first reservation, and this is the point where being wrong about
        # capacity costs an hour of compilation.
        self._storage.ensure_space("package")
        make_dir(self._dist)
        remove(self._record)

        signing = signing_key(self.root, self._config)
        environment = package_environment(
            self._config,
            self.target,
            toolchain=pinned_toolchain(self.root),
            manifest_url=self.manifest_url,
            signing=signing,
        )
        mount = self._config.install.mount
        metadata = docker_git_metadata_mount(self._runner)
        mounts = (
            # No source mount. The checkout is copied into the lane image
            # below, so the container holds its own bytes and a host step
            # cannot race these inodes -- and the bundler's atomic
            # temporaries, which made a read-only mount impossible, land in an
            # image layer instead of the developer's `frontend/`.
            *((metadata,) if metadata is not None else ()),
            *(Mount(volume.source, volume.target) for volume in self._package.volumes),
            Mount(
                self._package.target_volume_for(self.target.name),
                self._package.cargo_target_mount,
            ),
        )

        docker = Docker(self._runner)
        container = self._package.lane_container.format(arch=self.target.name)
        # Any predecessor first: a container left by a killed run holds the
        # name this one needs, and `docker create` fails on the collision
        # rather than replacing it.
        docker.remove(container)
        # The source as a layer. Built here rather than in a warm-up step
        # because it is keyed by the source itself: `COPY` invalidates on any
        # change, so a stale image is not reachable.
        docker.build(
            tag=self._package.lane_image,
            dockerfile=str(self.root / self._package.lane_dockerfile),
            context=str(self.root),
            args=[f"BASE={self._package.builder_image}"],
        )
        docker.create(
            name=container,
            image=self._package.lane_image,
            command=["bash", f"{mount}/{self._package.build_script}"],
            network=self._package.network,
            # Whatever signing contributed is the credential set, taken from
            # the keys it returned rather than from a second list of names.
            forward=tuple(environment),
            carry=environment,
            mounts=mounts,
            scratch=tuple(f"{mount}/{path}" for path in self._package.writable_paths),
            workdir=mount,
            secret_env=frozenset(signing),
        )
        try:
            docker.start(container)
        finally:
            # Before the removal, and on the failure path too: a build that
            # failed after producing a package is exactly when the package is
            # worth looking at, and `--rm` would have destroyed it. This is
            # also what makes "the builder produced it" and "the host can read
            # it" two events instead of one write through a shared mount.
            docker.copy_out(container, self._package.container_output_contents, str(self._dist))
            docker.remove(container)

    def resolve(self) -> Path:
        """The exact package this run produced, not whatever `dist/` holds.

        The builder writes the basename it just created. Globbing `dist/`
        instead would happily prove and publish a package left by an earlier
        build of a different commit.
        """
        if not self._record.is_file() or not self._record.read_text().strip():
            raise GateError("builder did not record the exact Debian package")
        name = self._record.read_text(encoding="utf-8").strip()

        if not name.endswith(self._package.package_suffix):
            raise GateError(f"invalid Debian package record: {name}")
        if name != Path(name).name:
            raise GateError(f"Debian package record escaped {self._package.dist_dir}/: {name}")

        package = self._dist / name
        if not package.is_file():
            raise GateError(f"recorded Debian package is missing: {package}")
        return package

    # -- after the build ---------------------------------------------------

    def prove(self) -> None:
        package = self.resolve()
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

    def collect(self) -> None:
        """List what this lane produced, then give its disk back."""
        self._runner.step("Artifacts")
        self._runner.run(["ls", "-lh", str(self._dist)])
        remove(self._record)
        self._storage.gc()
