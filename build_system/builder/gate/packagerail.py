"""One architecture's Linux release package, built in the sealed package helper.

Native cross-compilation, no QEMU. The image runs on the host architecture and
targets the other through `--target` plus multiarch system libraries. An
explicit network-open step materializes the locked Cargo/pnpm graphs and exact
ORT archive into a per-target helper; the source image and runtime consume its
exact OCI ID with networking disabled. `CARGO_TARGET_DIR=/cargo-target` inside
the container keeps first-party build output out of both the helper and host.

`.github/workflows/release.yaml`'s `build-app-linux` job calls this same rail;
CI takes its Tauri signing keys from secrets where a local build takes them
from `private/tauri/`.

The build itself is `build_system/packaging/linux/build-linux-package.sh`. It used to be the
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
from . import debproof, host, packagebuilder
from .config import Arch
from .content import ProfileContent
from .docker import Docker
from .dockermount import Mount
from .errors import GateError
from .fileactions import make_dir, remove
from .invocation import ConsoleMode
from .packageinputs import package_environment, pinned_toolchain, resolve_channel
from .packagesigning import signing_key
from .proc import Runner
from .sourcecommit import source_commit_for_checkout
from .storage import Storage


class PackageRail:
    """Builds, records, and optionally proves one architecture's package."""

    def __init__(
        self,
        runner: Runner,
        target: Arch,
        *,
        content: ProfileContent,
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
        self.content = content
        self.manifest_url = manifest_url or self._package.default_manifest_url
        self.channel = resolve_channel(channel or self._package.default_channel, self._config)
        self._require_proof = require_proof

    @property
    def _packages(self) -> Path:
        return self._config.path(self._config.outputs.packages)

    @property
    def _record(self) -> Path:
        return self._packages / f".cross-compile-{self.target.name}-deb"

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

    def require_content(self) -> None:
        """Prove the paired bundle only for the target this job can build."""
        self.content.require_complete(self._config, arches=(self.target,))

    def materialize(self) -> packagebuilder.PackageBuilderIdentity:
        """Resolve package dependencies at the graph's network-open edge."""
        return packagebuilder.materialize(self._runner, self._config, self.target)

    # -- the build ---------------------------------------------------------

    def build(self) -> None:
        self._runner.step(
            f"Building Linux deb ({self.target.name} via docker, target={self.target.rust_target})"
        )
        # Again, here: the builder image and the asset sync have landed since
        # the first reservation, and this is the point where being wrong about
        # capacity costs an hour of compilation.
        self._storage.ensure_space("package")
        make_dir(self._packages)
        remove(self._record)

        signing = signing_key(self.root, self._config)
        environment = package_environment(
            self._config,
            self.target,
            toolchain=pinned_toolchain(self.root),
            manifest_url=self.manifest_url,
            signing=signing,
            # From the tree being built, which under a prefix is the private
            # copy -- the same revision `source.record` measured, not whatever
            # the developer's checkout has moved on to since.
            revision=self._runner.capture(
                ["git", "-C", str(self.root), "rev-parse", "--short", "HEAD"]
            ),
        )
        mount = self._config.install.mount
        assets_destination, config_destination = self._package.generated_inputs
        mounts = (
            # No source mount. The checkout is copied into the lane image
            # below, so the container holds its own bytes and a host step
            # cannot race these inodes -- and the bundler's atomic
            # temporaries, which made a read-only mount impossible, land in an
            # image layer instead of the developer's `frontend/`.
            # The two generated trees the build reads. Mounted, not copied:
            # see `Mount.generated` -- `assets/` alone is 3.0 GB and changes
            # every run, so copying it would put a multi-gigabyte layer in
            # Docker storage per gate to avoid a mount that was never the race.
            Mount.generated(str(self.content.assets), f"{mount}/{assets_destination}"),
            Mount.generated(str(self.content.config), f"{mount}/{config_destination}"),
        )

        docker = Docker(self._runner)
        helper_local_image = packagebuilder.require_local_image(
            self._runner, self._config, self.target
        )
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
            args=[f"BASE={helper_local_image}"],
            platform=self._config.host_arch().docker_platform,
            network=self._package.builder.source_build_network,
            console=ConsoleMode.LOG_ONLY,
        )
        # A local FROM tag is portable across Docker and Colima; bind it to
        # the same exact helper image on both sides of the sealed build.
        packagebuilder.require_local_image(self._runner, self._config, self.target)
        docker.create(
            name=container,
            image=self._package.lane_image,
            command=["bash", f"{mount}/{self._package.build_script}"],
            network=self._package.builder.runtime_network,
            # Whatever signing contributed is the credential set, taken from
            # the keys it returned rather than from a second list of names.
            forward=tuple(environment),
            carry=environment,
            mounts=mounts,
            # The build directory joins the scratch grafts rather than becoming a
            # `Mount` with no source: an anonymous volume is spelled by its
            # container path alone, and `scratch` already spells them that way.
            # It was a named per-arch volume, which was the last state two
            # gates shared.
            scratch=(
                *(f"{mount}/{path}" for path in self._package.writable_paths),
                self._package.cargo_target_mount,
            ),
            workdir=mount,
            secret_env=frozenset(signing),
        )
        try:
            docker.start(container, console=ConsoleMode.LOG_ONLY)
        finally:
            # Before the removal, and on the failure path too: a build that
            # failed after producing a package is exactly when the package is
            # worth looking at, and `--rm` would have destroyed it. This is
            # also what makes "the builder produced it" and "the host can read
            # it" two events instead of one write through a shared mount.
            docker.copy_out(
                container,
                self._package.container_output_contents,
                str(self._packages),
            )
            docker.remove(container)

    def resolve(self) -> Path:
        """The exact package this run produced, not whatever `target/packages/` holds.

        The builder writes the basename it just created. Globbing the package
        output root instead would happily prove and publish a package left by
        an earlier build of a different commit.
        """
        if not self._record.is_file() or not self._record.read_text().strip():
            raise GateError("builder did not record the exact Debian package")
        name = self._record.read_text(encoding="utf-8").strip()

        if not name.endswith(self._package.package_suffix):
            raise GateError(f"invalid Debian package record: {name}")
        if name != Path(name).name:
            raise GateError(
                f"Debian package record escaped {self._config.outputs.packages}/: {name}"
            )

        package = self._packages / name
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
            content=self.content,
            manifest_url=self.manifest_url,
            channel=self.channel,
            source_commit=source_commit_for_checkout(self.root),
        ).run()

    def defer_proof(self) -> None:
        """Leave the exact native install to the complete local transaction.

        The complete candidate later authors a checked release graph from the
        assets it just built, hands that graph to this exact package, and runs
        the broader install, Winterfell, and glow-up proof. Repeating the
        narrower proof here would instead hydrate from mutable public stable,
        making a broken channel impossible to repair through a release command.
        Standalone package rails still call :meth:`prove`.
        """
        self._runner.note(
            "Exact package proof is owned by the later local authoritative install transaction."
        )

    def collect(self) -> None:
        """List what this lane produced, then give its disk back."""
        self._runner.step("Artifacts")
        self._runner.run(["ls", "-lh", str(self._packages)])
        remove(self._record)
        self._storage.gc()
