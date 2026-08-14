"""Install one exact `.deb` into sealed systemd and boot a real guest shell."""

from __future__ import annotations

import re
import time
from pathlib import Path

from . import config as gate_config
from . import installimage
from .content import ProfileContent
from .docker import Docker, Mount
from .errors import GateError
from .installcontainer import (
    VmDeviceRuntime,
    await_systemd,
    systemd_command,
    virtualisation_runtime,
)
from .installproof import InstallProof
from .proc import Runner
from .releasegraph import ReleaseGraph
from .sourcecommit import SourceCommit

PROFILE_READY = re.compile(r"^Profiles:\s+(\d+)/(\d+) ready", re.M)


class DebProof:
    """One clean-container installation of one exact Debian package."""

    def __init__(
        self,
        runner: Runner,
        *,
        package: Path,
        content: ProfileContent,
        manifest_url: str,
        channel: str,
        source_commit: SourceCommit,
        sleep=time.sleep,
    ) -> None:
        self._runner = runner
        self._docker = Docker(runner)
        self._config = gate_config.for_root(runner.root)
        self._proof = self._config.package.proof
        self._install = self._config.install
        self._staging = InstallProof(
            runner,
            self._config,
            source_commit=source_commit,
            container=self._proof.container,
            sleep=sleep,
        )
        self._graph = ReleaseGraph(
            self._docker,
            self._config,
            source_commit=source_commit,
            container=self._proof.container,
        )
        self._content = content
        self.root = self._config.root
        self.package = self._resolve(package)
        self.manifest_url = manifest_url
        if channel not in self._config.package.channels:
            raise GateError(f"unsupported exact package proof channel: {channel}")
        self.channel = channel
        self._sleep = sleep

    def _resolve(self, package: Path) -> Path:
        """Only a package this checkout built, named absolutely.

        `dist/*.deb` is the whole accepted set: anything else is a package
        whose provenance this proof cannot speak for.
        """
        resolved = Path(package).resolve()
        expected = self.root / self._config.package.dist_dir
        suffix = self._config.package.package_suffix
        if resolved.parent != expected or resolved.suffix != suffix:
            raise GateError(
                f"exact Debian package proof only accepts "
                f"{expected.name}/*{suffix} (got: {resolved})"
            )
        if not resolved.is_file():
            raise GateError(f"exact Debian package is missing: {resolved}")
        return resolved

    def run(self) -> None:
        self._content.require_complete(
            self._config,
            arches=(self._config.host_arch(),),
        )
        runtime: VmDeviceRuntime = virtualisation_runtime(
            self._install,
            purpose="the exact Debian package proof needs KVM and vhost-vsock",
        )
        container_deb = f"{self._install.mount}/{self.package.relative_to(self.root)}"
        expected = self._runner.capture(["dpkg-deb", "-f", str(self.package), "Version"])
        if not expected:
            raise GateError(f"{self.package.name} declares no Version field")

        try:
            self._start(runtime)
            self._staging.stage_content_from(
                assets=self._install.proof_assets_mount,
                content_config=self._install.proof_config_mount,
            )
            self._prepare_handoff(container_deb, expected)
            self._install_package(container_deb, expected)
            self._require_binaries(expected)
            ready, total = self._require_status()
            self._verify_release(expected)
            self._prove_shell()
        finally:
            self._graph.clear_handoff()
            self._docker.remove(self._proof.container)

        self._runner.note(
            f"Exact Debian package proof passed: version={expected} profiles={ready}/{total}"
        )

    def _start(self, runtime: VmDeviceRuntime) -> None:
        self._runner.note("Starting clean systemd container for exact package proof...")
        cgroup = self._install.cgroup_path
        image = installimage.require_local_image(self._runner, self._config)
        tmpfs = [f for path in self._install.tmpfs_paths for f in ("--tmpfs", path)]
        self._docker.remove(self._proof.container)
        self._docker.run_detached(
            network=self._install.runtime_network,
            name=self._proof.container,
            image=image,
            command=systemd_command(self._install, runtime.runtime_user_devices),
            options=[
                "--privileged",
                "--cgroupns=host",
                "--security-opt",
                "seccomp=unconfined",
                *runtime.docker_options,
                *tmpfs,
            ],
            mounts=[
                Mount(cgroup, cgroup, "rw"),
                # Generated after the image; read-only so proof cannot alter it.
                *(
                    Mount.generated(str(self.root / name), f"{self._install.mount}/{name}")
                    for name in self._install.generated_inputs
                    if (self.root / name).exists()
                ),
                Mount.generated(
                    str(self._content.assets),
                    self._install.proof_assets_mount,
                ),
                Mount.generated(
                    str(self._content.config),
                    self._install.proof_config_mount,
                ),
            ],
        )
        await_systemd(
            self._docker,
            self._proof.container,
            attempts=self._proof.systemd_ready_attempts,
            interval=self._install.systemd_ready_interval_seconds,
            sleep=self._sleep,
        )
        user = self._install.guest_user.name
        for device in self._install.vm_devices:
            self._docker.exec(
                self._proof.container,
                ["test", "-r", device, "-a", "-w", device],
                user=user,
            )

    def _prepare_handoff(self, package: str, version: str) -> None:
        """Author the exact local graph before the package's postinst runs."""
        layout = self._install.layout
        self._graph.author_exact_package(
            package=package,
            version=version,
            assets_manifest=f"{layout.assets}/{self._install.manifest_name}",
            candidate_base=f"{self._install.mount}/{layout.packages}",
            assets_dir=layout.assets,
            profiles_dir=f"{layout.config}/{self._config.functional.profiles_subdir}",
            channel=self.channel,
            profile_revision_policy=self._install.profile_revision_policy,
            manifest_version=self._install.manifest_version,
            out_dir=layout.channel,
        )

    def _install_package(self, container_deb: str, expected: str) -> None:
        self._runner.note(f"Installing exact package: {self.package}")
        self._staging.verify_package_dependencies(container_deb)
        self._docker.shell(
            self._proof.container,
            f'dpkg -i "{container_deb}"',
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
        bin_dir = self._install.bin_dir
        for name in self._proof.binaries:
            self._docker.exec(self._proof.container, ["test", "-x", f"{bin_dir}/{name}"])
        for name in self._proof.versioned_binaries:
            reported = self._docker.capture(
                self._proof.container, [f"{bin_dir}/{name}", "--version"]
            )
            if expected not in reported:
                raise GateError(
                    f"{bin_dir}/{name} reports {reported!r}, which does not carry the "
                    f"package version {expected}"
                )

    def _guest_env(self) -> dict[str, str]:
        guest = self._install.guest_user
        return {"HOME": guest.home, "XDG_RUNTIME_DIR": guest.runtime_dir}

    def _require_status(self) -> tuple[int, int]:
        """`capsem status` from the installed package, as the user would run it."""
        guest = self._install.guest_user
        output = self._docker.capture(
            self._proof.container,
            [self._install.installed_capsem, "status"],
            user=guest.name,
            env=self._guest_env(),
        )
        self._runner.note(output)

        missing = [line for line in self._proof.status_requires if line not in output]
        if missing:
            raise GateError("the installed package's status is missing: " + ", ".join(missing))

        counts = PROFILE_READY.search(output)
        if counts is None:
            raise GateError("exact package status has no Profiles: ready count")
        ready, total = int(counts.group(1)), int(counts.group(2))
        if total <= 0 or ready != total:
            raise GateError(f"exact package profiles are not all ready: {ready}/{total}")
        return ready, total

    def _verify_release(self, expected: str) -> None:
        manifest = self._graph.handed_off
        if manifest is None:
            raise GateError("exact Debian package proof has no authoritative manifest handoff")
        guest = self._install.guest_user
        self._docker.exec(
            self._proof.container,
            [
                "python3",
                f"{self._install.mount}/{self._proof.verify_script}",
                "--capsem",
                self._install.installed_capsem,
                "--capsem-home",
                f"{guest.home}/{self._install.capsem_home}",
                "--manifest-url",
                manifest,
                "--channel",
                self.channel,
                "--package-version",
                expected,
            ],
            user=guest.name,
            env=self._guest_env(),
        )

    def _prove_shell(self) -> None:
        guest = self._install.guest_user
        self._docker.exec(
            self._proof.container,
            [
                "python3",
                f"{self._install.mount}/{self._proof.shell_proof_script}",
                "--capsem",
                self._install.installed_capsem,
                "--marker",
                self._proof.shell_marker,
                "--session-name",
                self._proof.session_name,
                "--timeout",
                str(self._proof.shell_timeout_seconds),
            ],
            user=guest.name,
            env=self._guest_env(),
        )
