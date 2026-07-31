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

from . import config as gate_config
from . import host
from .docker import Docker, Mount
from .errors import GateError
from .proc import Runner


class InstallContainer:
    """A systemd container, its host prerequisites, and its file ownership."""

    def __init__(self, runner: Runner, *, sleep=time.sleep) -> None:
        self._runner = runner
        self._docker = Docker(runner)
        self._config = gate_config.for_root(runner.root)
        self._settings = self._config.install
        self.name = self._settings.container
        self._owned = self._settings.layout.owned_paths(self._settings.mount)
        self._sleep = sleep
        self._rosetta_required = False
        self.boots_a_guest = False

    # -- host capability ---------------------------------------------------

    def runtime_options(self) -> list[str]:
        """Device flags for this host, and whether a guest VM can boot here."""
        options = ["--security-opt", "seccomp=unconfined"]
        if not host.on_linux():
            return options

        for device in self._settings.vm_devices:
            if not host.device_available(device):
                raise GateError(
                    "installed doctor requires KVM and vhost-vsock on the Linux "
                    f"runner; {device} is not readable and writable"
                )
            options += ["--device", device]
        for device in self._settings.optional_vm_devices:
            if host.device_available(device):
                options += ["--device", device]
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
        # A stable name plus a preemptive removal is what recovers from a
        # predecessor that died before its own cleanup -- a cargo SIGTERM under
        # a Colima OOM, for instance.
        self._docker.remove(self.name)
        self._docker.run_detached(
            name=self.name,
            image=self._settings.image,
            command=["/usr/lib/systemd/systemd"],
            options=["--privileged", "--cgroupns=host", *options,
                     "--tmpfs", "/run", "--tmpfs", "/tmp"],
            mounts=[
                Mount("/sys/fs/cgroup", "/sys/fs/cgroup", "rw"),
                Mount(str(self._config.root), self._settings.mount),
                *(Mount(v.source, v.target) for v in self._settings.volumes),
            ],
        )
        if self.boots_a_guest:
            for device in self._settings.vm_devices:
                self._docker.exec(self.name, ["test", "-r", device, "-a", "-w", device])
        self._await_systemd()
        self._claim_paths()

    def _await_systemd(self) -> None:
        await_systemd(
            self._docker,
            self.name,
            attempts=self._settings.systemd_ready_attempts,
            interval=self._settings.systemd_ready_interval_seconds,
            sleep=self._sleep,
        )

    def _claim_paths(self) -> None:
        self._docker.exec(self.name, ["mkdir", "-p", *self._owned])
        self._docker.exec(self.name, ["chown", "-R", "capsem:capsem", *self._owned])
        # Unlinking these needs write permission on the *parent* directory
        # entry, not on the entries themselves. On Linux /src/target belongs to
        # the host user rather than the container's capsem, so the staging
        # step's `rm -rf` fails with "Permission denied" before it can restage.
        # Grant the one directory entry: a recursive chown here would walk
        # every cargo artifact in the checkout.
        self._docker.exec(
            self.name, ["chown", "capsem:capsem", f"{self._settings.mount}/target"]
        )

    def return_paths(self) -> None:
        """Hand the container's writes back to the host user that owns them."""
        uid, gid = host.user()
        self._docker.exec(
            self.name, ["chown", "-R", f"{uid}:{gid}", *self._owned], check=False
        )

    def hand_back(self, path: str) -> None:
        """Return one host-owned path mid-run, before a host tool reads it."""
        uid, gid = host.user()
        self._docker.shell(self.name, f"mkdir -p {path} && chown -R {uid}:{gid} {path}")

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
        f"systemd never reached running or degraded in {container} after "
        f"{attempts * interval:.0f}s"
    )
