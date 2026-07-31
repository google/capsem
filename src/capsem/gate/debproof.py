"""Install the exact `.deb` into a clean container and prove it works.

Distinct from the install gate: that one authors a release graph and runs the
whole glow-up, while this answers a narrower question about a package the
cross-compile rail just produced -- does it install, does every binary it
claims to ship exist and report the version on the tin, does the service come
up, is every profile ready, and can it open a real guest shell.

The checkout is mounted read-only on purpose. This proof must not be able to
influence the tree it is proving, and a package that only works because it
wrote something back into `/src` is not a package that works.

The narrow assertion worth keeping in view is the version check on each binary.
A `.deb` can install cleanly while carrying binaries from an earlier build --
the package metadata and the ELF inside it are stamped separately -- and every
file-existence check in the world passes on that package.
"""

from __future__ import annotations

import argparse
import os
import re
import time
from pathlib import Path

from . import config as gate_config
from . import host
from .docker import Docker, Mount
from .errors import GateError
from .installcontainer import await_systemd
from .proc import Runner

# `Profiles: 3/3 ready`, whose two numbers must match and must not be zero.
PROFILE_READY = re.compile(r"^Profiles:\s+(\d+)/(\d+) ready", re.M)

class DebProof:
    """One clean-container installation of one exact Debian package."""

    def __init__(
        self,
        runner: Runner,
        *,
        package: Path,
        manifest_url: str,
        channel: str,
        sleep=time.sleep,
    ) -> None:
        self._runner = runner
        self._docker = Docker(runner)
        self._config = gate_config.for_root(runner.root)
        self._proof = self._config.package.proof
        self._install = self._config.install
        self.root = self._config.root
        self.package = self._resolve(package)
        self.manifest_url = manifest_url
        self.channel = self._resolve_channel(channel)
        self._sleep = sleep

    # -- inputs ------------------------------------------------------------

    def _resolve(self, package: Path) -> Path:
        """Only a package this checkout built, named absolutely.

        `dist/*.deb` is the whole accepted set: anything else is a package
        whose provenance this proof cannot speak for.
        """
        resolved = Path(package).resolve()
        expected = self.root / "dist"
        if resolved.parent != expected or resolved.suffix != ".deb":
            raise GateError(
                f"exact Debian package proof only accepts dist/*.deb (got: {resolved})"
            )
        if not resolved.is_file():
            raise GateError(f"exact Debian package is missing: {resolved}")
        return resolved

    def _resolve_channel(self, channel: str) -> str:
        if channel not in self._config.package.channels:
            raise GateError(f"unsupported exact package proof channel: {channel}")
        return channel

    def _require_virtualisation(self) -> list[str]:
        devices = []
        for device in self._install.vm_devices:
            if not host.device_available(device):
                raise GateError(
                    f"the exact Debian package proof needs {device}, which is not "
                    "readable and writable on this host"
                )
            devices += ["--device", device]
        for device in self._install.optional_vm_devices:
            if host.device_available(device):
                devices += ["--device", device]
        return devices

    # -- the run -----------------------------------------------------------

    def run(self) -> None:
        devices = self._require_virtualisation()
        container_deb = f"{self._install.mount}/{self.package.relative_to(self.root)}"
        expected = self._runner.capture(["dpkg-deb", "-f", str(self.package), "Version"])
        if not expected:
            raise GateError(f"{self.package.name} declares no Version field")

        try:
            self._start(devices)
            self._install_package(container_deb, expected)
            self._require_binaries(expected)
            ready, total = self._require_status()
            self._verify_release(expected)
            self._prove_shell()
        finally:
            self._docker.remove(self._proof.container)

        self._runner.note(
            f"Exact Debian package proof passed: version={expected} "
            f"profiles={ready}/{total}"
        )

    def _start(self, devices: list[str]) -> None:
        self._runner.note("Starting clean systemd container for exact package proof...")
        self._docker.remove(self._proof.container)
        self._docker.run_detached(
            name=self._proof.container,
            image=self._install.image,
            command=["/usr/lib/systemd/systemd"],
            options=["--privileged", "--cgroupns=host",
                     "--security-opt", "seccomp=unconfined", *devices,
                     "--tmpfs", "/run", "--tmpfs", "/tmp"],
            mounts=[
                Mount("/sys/fs/cgroup", "/sys/fs/cgroup", "rw"),
                # Read-only: this proof must not be able to influence the tree
                # it is proving.
                Mount(str(self.root), self._install.mount, "ro"),
            ],
        )
        await_systemd(
            self._docker,
            self._proof.container,
            attempts=self._proof.systemd_ready_attempts,
            interval=self._install.systemd_ready_interval_seconds,
            sleep=self._sleep,
        )
        for device in self._install.vm_devices:
            self._docker.exec(
                self._proof.container, ["test", "-r", device, "-a", "-w", device]
            )

    def _install_package(self, container_deb: str, expected: str) -> None:
        self._runner.note(f"Installing exact package: {self.package}")
        self._docker.shell(
            self._proof.container,
            f'dpkg -i "{container_deb}" 2>&1 || apt-get install -f -y',
        )
        name = self._install.suite.package_name
        state = self._docker.capture(
            self._proof.container, ["dpkg-query", "-W", "-f=${Status}", name]
        )
        version = self._docker.capture(
            self._proof.container, ["dpkg-query", "-W", "-f=${Version}", name]
        )
        if state != "install ok installed":
            raise GateError(f"dpkg reports {name} as {state!r}, not installed")
        if version != expected:
            raise GateError(f"dpkg installed {name} {version}, expected {expected}")

    def _require_binaries(self, expected: str) -> None:
        """Every binary present, and reporting the version on the package.

        A `.deb` can install cleanly carrying binaries from an earlier build:
        the package metadata and the ELF inside it are stamped separately, and
        a file-existence check passes either way.
        """
        for name in self._proof.binaries:
            self._docker.exec(self._proof.container, ["test", "-x", f"/usr/bin/{name}"])
        for name in self._proof.versioned_binaries:
            reported = self._docker.capture(
                self._proof.container, [f"/usr/bin/{name}", "--version"]
            )
            if expected not in reported:
                raise GateError(
                    f"/usr/bin/{name} reports {reported!r}, which does not carry the "
                    f"package version {expected}"
                )

    def _guest_env(self) -> dict[str, str]:
        guest = self._install.guest_user
        return {"HOME": guest.home, "XDG_RUNTIME_DIR": guest.runtime_dir}

    def _require_status(self) -> tuple[int, int]:
        """`capsem status` from the installed package, as the user would run it."""
        guest = self._install.guest_user
        output = self._docker.capture(
            self._proof.container, ["/usr/bin/capsem", "status"],
            user=guest.name, env=self._guest_env(),
        )
        self._runner.note(output)

        missing = [line for line in self._proof.status_requires if line not in output]
        if missing:
            raise GateError(
                "the installed package's status is missing: " + ", ".join(missing)
            )

        counts = PROFILE_READY.search(output)
        if counts is None:
            raise GateError("exact package status has no Profiles: ready count")
        ready, total = int(counts.group(1)), int(counts.group(2))
        if total <= 0 or ready != total:
            raise GateError(
                f"exact package profiles are not all ready: {ready}/{total}"
            )
        return ready, total

    def _verify_release(self, expected: str) -> None:
        guest = self._install.guest_user
        self._docker.exec(
            self._proof.container,
            [
                "python3", f"{self._install.mount}/{self._proof.verify_script}",
                "--capsem", "/usr/bin/capsem",
                "--capsem-home", f"{guest.home}/.capsem",
                "--manifest-url", self.manifest_url,
                "--channel", self.channel,
                "--package-version", expected,
            ],
            user=guest.name,
            env=self._guest_env(),
        )

    def _prove_shell(self) -> None:
        guest = self._install.guest_user
        self._docker.exec(
            self._proof.container,
            [
                "python3", f"{self._install.mount}/{self._proof.shell_proof_script}",
                "--capsem", "/usr/bin/capsem",
                "--marker", self._proof.shell_marker,
                "--session-name", self._proof.session_name,
                "--timeout", str(self._proof.shell_timeout_seconds),
            ],
            user=guest.name,
            env=self._guest_env(),
        )


def register(subparsers: argparse._SubParsersAction) -> None:
    parser = subparsers.add_parser(
        "prove-deb", help="install one exact dist/*.deb in a clean container and prove it"
    )
    parser.set_defaults(handler=_command)


def _required(name: str) -> str:
    value = os.environ.get(name)
    if not value:
        raise GateError(f"exact package proof requires {name}")
    return value


def _command(args: argparse.Namespace, runner: Runner) -> int:
    DebProof(
        runner,
        package=Path(_required("CAPSEM_PROOF_DEB")),
        manifest_url=_required("CAPSEM_PROOF_MANIFEST_URL"),
        channel=_required("CAPSEM_PROOF_MANIFEST_CHANNEL"),
    ).run()
    return 0
