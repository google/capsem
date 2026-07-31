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

import argparse
import os
import tomllib
from pathlib import Path

from . import config as gate_config
from . import host
from .config import Arch
from .errors import GateError
from .proc import Runner
from .storage import Storage


def pinned_toolchain(root: Path) -> str:
    """The Rust version `rust-toolchain.toml` pins, read rather than repeated.

    It was spelled three times inside one inline shell script, which is three
    chances for a toolchain bump to leave the package rail behind.
    """
    pin = Path(root) / "rust-toolchain.toml"
    try:
        return tomllib.loads(pin.read_text(encoding="utf-8"))["toolchain"]["channel"]
    except (OSError, KeyError, tomllib.TOMLDecodeError) as exc:
        raise GateError(f"{pin} declares no [toolchain] channel: {exc}") from None


def signing_key(root: Path) -> dict[str, str]:
    """The real Tauri release keys, if this checkout has them.

    Absent, the container generates a throwaway dev key so `cargo tauri build`
    can finish. The authoritative keys live in GitHub Actions secrets and are
    applied only on publish.
    """
    private = Path(root) / "private" / "tauri" / "capsem.key"
    password = Path(root) / "private" / "tauri" / "password.txt"
    if not (private.is_file() and password.is_file()):
        return {}
    return {
        "TAURI_SIGNING_PRIVATE_KEY": private.read_text(encoding="utf-8"),
        "TAURI_SIGNING_PRIVATE_KEY_PASSWORD": password.read_text(encoding="utf-8"),
    }


def resolve_channel(channel: str, config: gate_config.GateConfig) -> str:
    allowed = config.package.channels
    if channel not in allowed:
        raise GateError(
            f"CAPSEM_INSTALL_CHANNEL must be one of {', '.join(allowed)} (got: {channel})"
        )
    return channel


class PackageRail:
    """Builds, records, and optionally proves one architecture's package."""

    def __init__(
        self,
        runner: Runner,
        target: Arch,
        *,
        manifest_url: str | None = None,
        channel: str = "stable",
        require_proof: bool = False,
    ) -> None:
        self._runner = runner
        self._config = gate_config.for_root(runner.root)
        self._package = self._config.package
        self._storage = Storage(runner)
        self.root = runner.root
        self.target = target
        self.manifest_url = manifest_url or self._package.default_manifest_url
        self.channel = resolve_channel(channel, self._config)
        self._require_proof = require_proof

    @property
    def _record(self) -> Path:
        return self.root / "dist" / f".cross-compile-{self.target.name}-deb"

    def run(self) -> Path:
        self._prepare_builder()
        self._sync_assets_for_tauri()
        self._build()
        package = self._recorded_package()
        self._prove(package)

        self._runner.step("Artifacts")
        self._runner.run(["ls", "-lh", str(self.root / "dist")])
        self._storage.gc()
        return package

    # -- before the build --------------------------------------------------

    def _prepare_builder(self) -> None:
        self._storage.release("completed-docker-rails")
        # A completed install target retains only top-level runtime binaries
        # and the previous package after its post-install purge, so it cannot
        # accelerate this build. Release it before sacrificing the reusable
        # builder image or registries.
        self._storage.release("deferred-install-target")
        # One named policy owns this rail: the Rust base image and the BuildKit
        # cohort stay warm across candidates, and a capacity failure reports an
        # explicit disk recommendation instead of silently building cold.
        self._storage.ensure_space("package")
        # Always run the cached image build, so a change to the Dockerfile or
        # its helpers cannot hide behind a stale local image.
        self._runner.run(["just", "_build-host-image"])
        self._storage.ensure_space("package")
        if host.on_macos():
            # Colima's VM clock drifts, and apt rejects a repository signed in
            # what it believes is the future.
            self._runner.run(["python3", "scripts/sync-container-clock.py"])

    def _sync_assets_for_tauri(self) -> None:
        """`assets/current` is what the bundler embeds; point it at this target."""
        current = self.root / "assets" / "current"
        built = self.root / "assets" / self.target.name
        self._runner.run(["rm", "-rf", str(current)])
        if built.is_dir():
            self._runner.run(["cp", "-r", str(built), str(current)])

    # -- the build ---------------------------------------------------------

    def _build(self) -> None:
        self._runner.step(
            f"Building Linux deb ({self.target.name} via docker, "
            f"target={self.target.rust_target})"
        )
        (self.root / "dist").mkdir(exist_ok=True)
        self._record.unlink(missing_ok=True)

        environment = {
            "TARGET_ARCH": self.target.name,
            "RUST_TARGET": self.target.rust_target,
            "DPKG_ARCH": self.target.dpkg,
            "RUST_TOOLCHAIN": pinned_toolchain(self.root),
            "PKG_CONFIG_PATH": self.target.pkg_config_path,
            "CAPSEM_INSTALL_MANIFEST_URL": self.manifest_url,
            "HOST_UID": str(host.user()[0]),
            "HOST_GID": str(host.user()[1]),
            **signing_key(self.root),
        }
        argv = ["docker", "run", "--rm"]
        for name, value in environment.items():
            argv += ["-e", f"{name}={value}"]
        argv += ["-v", f"{self.root}:/src"]
        for volume in self._package.volumes:
            argv += ["-v", f"{volume.source}:{volume.target}"]
        argv += [
            "-v", f"{self._package.target_volume_for(self.target.name)}:/cargo-target",
            "-w", "/src",
            self._package.builder_image,
            "bash", f"/src/{self._package.build_script}",
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

        if not name.endswith(".deb"):
            raise GateError(f"invalid Debian package record: {name}")
        if name != Path(name).name:
            raise GateError(f"Debian package record escaped dist/: {name}")

        package = self.root / "dist" / name
        if not package.is_file():
            raise GateError(f"recorded Debian package is missing: {package}")
        return package

    # -- after the build ---------------------------------------------------

    def _prove(self, package: Path) -> None:
        native = self._config.host_arch()
        kvm_ready = all(
            host.device_available(device) for device in self._config.install.vm_devices
        )
        decision = self._runner.capture(
            [
                "bash", str(self.root / self._package.proof_selector),
                host.system(), native.name, self.target.name,
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
        self._runner.run(
            ["just", "_prove-linux-deb"],
            env={
                "CAPSEM_PROOF_DEB": str(package),
                "CAPSEM_PROOF_MANIFEST_URL": self.manifest_url,
                "CAPSEM_PROOF_MANIFEST_CHANNEL": self.channel,
            },
        )


def register(subparsers: argparse._SubParsersAction) -> None:
    parser = subparsers.add_parser(
        "cross-compile", help="build the Linux release package for one architecture"
    )
    parser.add_argument("arch", nargs="?", help="arm64 or x86_64; defaults to the host")
    parser.set_defaults(handler=_command)


def _command(args: argparse.Namespace, runner: Runner) -> int:
    config = gate_config.for_root(runner.root)
    target = config.arch(args.arch) if args.arch else config.host_arch()
    PackageRail(
        runner,
        target,
        manifest_url=os.environ.get("CAPSEM_INSTALL_MANIFEST_URL"),
        channel=os.environ.get("CAPSEM_INSTALL_CHANNEL", "stable"),
        require_proof=os.environ.get("CAPSEM_REQUIRE_LINUX_DEB_PROOF", "0") == "1",
    ).run()
    return 0
