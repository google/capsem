"""The privileged systemd container the install proof runs inside.

Two host facts decide what this container can prove, and both were buried in
the middle of a 270-line recipe. On Linux with `/dev/kvm` and
`/dev/vhost-vsock` it boots a real guest and the glow-up is complete; without
them the run is a Linux packaging proof only, and saying which one happened is
the difference between a passing gate and a meaningful one.

Rosetta is checked twice on purpose. A privileged systemd container has
removed Colima's binfmt registration before now, and that breaks every later
x86 build on the machine rather than only this run -- so the second check
attributes the damage to the thing that caused it.
"""

from __future__ import annotations

import os
import shutil
import time
from pathlib import Path

from . import arch as architectures
from .docker import Docker, Mount
from .errors import GateError
from .proc import Runner


SYSTEMD_READY_ATTEMPTS = 30
SYSTEMD_READY_INTERVAL = 0.5

VM_DEVICES = ("/dev/kvm", "/dev/vhost-vsock")
ROSETTA_BINFMT = "/proc/sys/fs/binfmt_misc/rosetta"


class InstallContainer:
    """A systemd container, its host prerequisites, and its file ownership."""

    def __init__(
        self,
        runner: Runner,
        *,
        name: str,
        image: str,
        owned_paths: tuple[str, ...],
        sleep=time.sleep,
    ) -> None:
        self._runner = runner
        self._docker = Docker(runner)
        self.name = name
        self._image = image
        self._owned = owned_paths
        self._sleep = sleep
        self._rosetta_required = False
        self.boots_a_guest = False

    # -- host capability ---------------------------------------------------

    def runtime_options(self) -> list[str]:
        """Device flags for this host, and whether a guest VM can boot here."""
        options = ["--security-opt", "seccomp=unconfined"]
        if not architectures.on_linux():
            return options

        for device in VM_DEVICES:
            if not os.access(device, os.R_OK | os.W_OK):
                raise GateError(
                    "installed doctor requires KVM and vhost-vsock on the Linux "
                    f"runner; {device} is not readable and writable"
                )
            options += ["--device", device]
        if os.access("/dev/vsock", os.R_OK | os.W_OK):
            options += ["--device", "/dev/vsock"]
        self.boots_a_guest = True
        return options

    def _rosetta_registered(self) -> bool:
        return self._runner.succeeds(["colima", "ssh", "--", "test", "-f", ROSETTA_BINFMT])

    def require_rosetta(self) -> None:
        if not architectures.on_macos() or shutil.which("colima") is None:
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

    def start(self, *, root: Path, options: list[str], mounts: list[Mount]) -> None:
        self._runner.note("Starting systemd container...")
        # A stable name plus a preemptive removal is what recovers from a
        # predecessor that died before its own cleanup -- a cargo SIGTERM under
        # a Colima OOM, for instance.
        self._docker.remove(self.name)
        self._docker.run_detached(
            name=self.name,
            image=self._image,
            command=["/usr/lib/systemd/systemd"],
            options=["--privileged", "--cgroupns=host", *options,
                     "--tmpfs", "/run", "--tmpfs", "/tmp"],
            mounts=[Mount("/sys/fs/cgroup", "/sys/fs/cgroup", "rw"),
                    Mount(str(root), "/src"), *mounts],
        )
        if self.boots_a_guest:
            for device in VM_DEVICES:
                self._docker.exec(self.name, ["test", "-r", device, "-a", "-w", device])
        self._await_systemd()
        self._claim_paths()

    def _await_systemd(self) -> None:
        for _ in range(SYSTEMD_READY_ATTEMPTS):
            state = self._docker.shell_capture(
                self.name, "systemctl is-system-running --wait 2>/dev/null || true"
            )
            if "running" in state or "degraded" in state:
                return
            self._sleep(SYSTEMD_READY_INTERVAL)
        raise GateError(
            f"systemd never reached running or degraded in {self.name} after "
            f"{SYSTEMD_READY_ATTEMPTS * SYSTEMD_READY_INTERVAL:.0f}s"
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
        self._docker.exec(self.name, ["chown", "capsem:capsem", "/src/target"])

    def return_paths(self) -> None:
        """Hand the container's writes back to the host user that owns them."""
        self._docker.exec(
            self.name,
            ["chown", "-R", f"{os.getuid()}:{os.getgid()}", *self._owned],
            check=False,
        )

    def hand_back(self, path: str) -> None:
        """Return one host-owned path mid-run, before a host tool reads it."""
        self._docker.shell(
            self.name,
            f"mkdir -p {path} && chown -R {os.getuid()}:{os.getgid()} {path}",
        )

    def stop(self) -> None:
        self._docker.remove(self.name)
