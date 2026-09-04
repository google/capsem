"""The privileged systemd container the install proof runs inside.

Two host facts decide what this container can prove, and both were buried in
the middle of a 270-line recipe. On Linux with `/dev/kvm` and
`/dev/vhost-vsock` it boots a real guest and the glow-up is complete; without
them the run is a Linux packaging proof only, and saying which one happened is
the difference between a passing gate and a meaningful one.

Device names, timeouts, and the container's identity are `[install]` in
`config/gate.toml`.

Rosetta is checked twice on purpose. A privileged systemd container has
removed Colima's binfmt registration before now, and that breaks every later
x86 build on the machine rather than only this run -- so the second check
attributes the damage to the thing that caused it.
"""

from __future__ import annotations

import shutil
import time
from dataclasses import dataclass

from . import config as gate_config
from . import host, installimage
from .content import InstallContent, SelectedInstallContent
from .docker import Docker, Mount
from .errors import GateError
from .proc import Runner
from .productschema import InstallConfig


@dataclass(frozen=True)
class VmDeviceRuntime:
    """One indivisible device pass-through and runtime-user device selection."""

    docker_options: tuple[str, ...]
    runtime_user_devices: tuple[str, ...]


def virtualisation_runtime(settings: InstallConfig, *, purpose: str) -> VmDeviceRuntime:
    """Pass VM devices and name the required subset for runtime-user setup."""
    options: list[str] = []
    selected: list[str] = []
    for device in settings.vm_devices:
        if not host.device_available(device):
            raise GateError(f"{purpose}; {device} is not readable and writable")
        selected.append(device)
    for device in settings.optional_vm_devices:
        if host.device_available(device):
            selected.append(device)
    for device in selected:
        options += ["--device", device]
    return VmDeviceRuntime(tuple(options), settings.vm_devices)


def systemd_command(settings: InstallConfig, devices: tuple[str, ...]) -> list[str]:
    """Start systemd before postinstall grants current-session device access."""
    return [
        "bash",
        settings.vm_device_setup_script,
        settings.guest_user.name,
        settings.systemd_command,
        *devices,
    ]


def verify_vm_device_access(docker: Docker, container: str, settings: InstallConfig) -> None:
    """Prove package postinstall repaired the stale manager's device access."""
    user = settings.guest_user.name
    for device in settings.vm_devices:
        docker.exec(container, ["test", "-r", device, "-a", "-w", device], user=user)


class InstallContainer:
    """A systemd container, its host prerequisites, and its file ownership."""

    def __init__(
        self,
        runner: Runner,
        *,
        content: InstallContent | None = None,
        sleep=time.sleep,
    ) -> None:
        self._runner = runner
        self._docker = Docker(runner)
        self._config = gate_config.for_root(runner.root)
        self._settings = self._config.install
        self._content = content
        self.name = self._settings.container
        self._owned = self._settings.layout.owned_paths(self._settings.mount)
        self._sleep = sleep
        self._rosetta_required = False
        self._runtime_user_devices: tuple[str, ...] = ()
        self.boots_a_guest = False

    # -- host capability ---------------------------------------------------

    def runtime_options(self) -> list[str]:
        """Device flags for this host, and whether a guest VM can boot here."""
        options = ["--security-opt", "seccomp=unconfined"]
        if not host.on_linux():
            return options

        runtime = virtualisation_runtime(
            self._settings,
            purpose="installed doctor requires KVM and vhost-vsock on the Linux runner",
        )
        options += runtime.docker_options
        self._runtime_user_devices = runtime.runtime_user_devices
        self.boots_a_guest = True
        return options

    def _rosetta_registered(self) -> bool:
        return self._runner.succeeds(
            ["colima", "ssh", "--", "test", "-f", self._settings.rosetta_binfmt]
        )

    def require_rosetta(self) -> None:
        if not host.on_macos() or shutil.which("colima") is None:
            return
        if not self._runner.succeeds(["colima", "status"]):
            return
        if not self._rosetta_registered():
            raise GateError(
                "Colima Rosetta binfmt registration is missing before test-install.\n"
                "Run the canonical bootstrap to repair it before testing packages."
            )
        self._rosetta_required = True

    def verify_rosetta_survived(self) -> None:
        if self._rosetta_required and not self._rosetta_registered():
            raise GateError(
                "systemd install container removed Colima's Rosetta binfmt registration"
            )

    # -- lifecycle ---------------------------------------------------------

    def start(self, *, options: list[str]) -> None:
        self._runner.note("Starting systemd container...")
        cgroup = self._settings.cgroup_path
        image = installimage.require_local_image(self._runner, self._config)
        # A stable name plus a preemptive removal is what recovers from a
        # predecessor that died before its own cleanup -- a cargo SIGTERM under
        # a Colima OOM, for instance.
        self._docker.remove(self.name)
        self._docker.run_detached(
            network=self._settings.runtime_network,
            name=self.name,
            image=image,
            command=systemd_command(self._settings, self._runtime_user_devices),
            options=["--privileged", "--cgroupns=host", *options, *self._tmpfs()],
            mounts=[
                Mount(cgroup, cgroup, "rw"),
                Mount.generated(
                    str(self._config.path(self._settings.layout.glowup_evidence)),
                    f"{self._settings.mount}/{self._settings.layout.glowup_evidence}",
                    "rw",
                ),
                # What the image could not carry: the package this proof
                # installs is written by an earlier step, long after the image
                # was built. Read-only, so the proof cannot alter the artifact
                # it exists to verify.
                *(
                    Mount.generated(str(self._config.root / name), f"{self._settings.mount}/{name}")
                    for name in self._settings.generated_inputs
                    if (self._config.root / name).exists()
                ),
                *self._content_mounts(),
            ],
        )
        self._await_systemd()
        self._claim_paths()

    def verify_vm_device_access(self) -> None:
        if self.boots_a_guest:
            verify_vm_device_access(self._docker, self.name, self._settings)

    def _content_mounts(self) -> tuple[Mount, ...]:
        if self._content is None:
            return ()
        mount = self._settings.mount
        profile = self._content.content
        mounts = [
            Mount.generated(
                str(profile.assets),
                f"{mount}/{self._config.functional.assets_dir}",
            ),
            Mount.generated(
                str(profile.config),
                f"{mount}/{self._config.functional.config_root}",
            ),
        ]
        if isinstance(self._content, SelectedInstallContent):
            # stage-release-test-inputs writes absolute file:// URLs. Mounting
            # the one paired root at the same address keeps those immutable
            # bytes resolvable without exposing the checkout or public egress.
            root = self._content.content.root.resolve()
            mounts.append(Mount.generated(str(root), str(root)))
        return tuple(mounts)

    def _await_systemd(self) -> None:
        await_systemd(
            self._docker,
            self.name,
            attempts=self._settings.systemd_ready_attempts,
            interval=self._settings.systemd_ready_interval_seconds,
            sleep=self._sleep,
        )

    def _claim_paths(self) -> None:
        guest = self._settings.guest_user.name
        self._docker.exec(self.name, ["mkdir", "-p", *self._owned])
        self._docker.exec(self.name, ["chown", "-R", f"{guest}:{guest}", *self._owned])
        # Removing an owned path needs write permission on its parent, not on
        # the path itself. Claim each config-derived parent entry without
        # recursively walking unrelated build output beneath it.
        parents = self._settings.layout.owned_parent_paths(self._settings.mount)
        self._docker.exec(
            self.name,
            ["chown", f"{guest}:{guest}", *parents],
        )

    def return_paths(self) -> None:
        """Hand the container's writes back to the host user that owns them."""
        uid, gid = host.user()
        self._docker.exec(self.name, ["chown", "-R", f"{uid}:{gid}", *self._owned], check=False)

    def hand_back(self, path: str) -> None:
        """Return one host-owned path mid-run, before a host tool reads it."""
        uid, gid = host.user()
        self._docker.shell(self.name, f"mkdir -p {path} && chown -R {uid}:{gid} {path}")

    def _tmpfs(self) -> list[str]:
        return [flag for path in self._settings.tmpfs_paths for flag in ("--tmpfs", path)]

    def stop(self) -> None:
        self._docker.remove(self.name)


def await_systemd(
    docker: Docker,
    container: str,
    *,
    attempts: int,
    interval: float,
    sleep=time.sleep,
) -> None:
    """Wait for systemd to finish coming up inside a container.

    `degraded` counts: a container with one failed unit still installs
    packages, and refusing it would fail the gate on something it does not
    test.
    """
    for _ in range(attempts):
        state = docker.shell_capture(
            container, "systemctl is-system-running --wait 2>/dev/null || true"
        )
        if "running" in state or "degraded" in state:
            return
        sleep(interval)
    raise GateError(
        f"systemd never reached running or degraded in {container} after {attempts * interval:.0f}s"
    )
