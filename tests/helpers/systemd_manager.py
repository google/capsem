"""An isolated real user manager for Linux supervisor acceptance."""

from __future__ import annotations

import os
import socket
import struct
import subprocess
import time
from collections.abc import Callable, Iterator
from contextlib import contextmanager
from dataclasses import dataclass, field
from pathlib import Path


@dataclass(frozen=True)
class UserManager:
    environment: dict[str, str] = field(repr=False)
    pid: int


def manager_pid(runtime: Path) -> int:
    with socket.socket(socket.AF_UNIX) as connection:
        connection.settimeout(2)
        connection.connect(str(runtime / "systemd/private"))
        pid, uid, _ = struct.unpack("3i", connection.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))
    assert uid == os.getuid() and pid > 0, "user manager has a different native owner"
    return pid


@contextmanager
def manager_environment(
    root: Path, label: str, retire: Callable[[], None],
    *, private_network: bool = False,
) -> Iterator[UserManager]:
    """Keep the manager's runtime, units and network in the test's ownership.

    Root delegates one transient manager to the test user. The manager joins
    the caller's network namespace before dropping privilege, so a sealed
    gate cannot accidentally launch its service into the host network.
    """
    if not private_network and os.environ.get("XDG_RUNTIME_DIR"):
        environment = dict(os.environ)
        manager = UserManager(environment, manager_pid(Path(environment["XDG_RUNTIME_DIR"])))
        slot = subprocess.run(
            ["systemctl", "--user", "show", "capsem.service", "--property=LoadState", "--value"],
            env=environment, capture_output=True, text=True, timeout=2, check=False,
        )
        assert slot.stdout.strip() == "not-found", "restart fixture cannot adopt an existing service unit"
        try:
            yield manager
        finally:
            retire()
        return
    runtime = root / "systemd-runtime"
    runtime.mkdir(mode=0o700)
    environment = {
        **os.environ,
        "XDG_RUNTIME_DIR": str(runtime),
        "DBUS_SESSION_BUS_ADDRESS": f"unix:path={runtime / 'bus'}",
    }
    unit = label.removesuffix(".service") + "-manager.service"
    privileged = [] if os.geteuid() == 0 else ["sudo", "-n"]

    def command(*args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [*privileged, *args], capture_output=True, text=True, timeout=15, check=False,
        )

    try:
        started = command(
            "systemd-run", "--collect", f"--unit={unit}",
            f"--property=User={os.getuid()}", "--property=Delegate=yes",
            *( ["--property=PrivateNetwork=yes"] if private_network else
               [f"--property=NetworkNamespacePath=/proc/{os.getpid()}/ns/net"] ),
            f"--setenv=XDG_RUNTIME_DIR={runtime}",
            "--setenv=SYSTEMD_UNIT_PATH=/usr/lib/systemd/user",
            "/usr/lib/systemd/systemd", "--user", "--unit=basic.target",
        )
        assert started.returncode == 0, started.stdout + started.stderr
        deadline = time.monotonic() + 15
        last_error = ""
        while time.monotonic() < deadline:
            ready = subprocess.run(
                ["systemctl", "--user", "show", "--property=Version", "--value"],
                env=environment, capture_output=True, text=True, timeout=2, check=False,
            )
            if ready.returncode == 0:
                break
            last_error = ready.stderr
            time.sleep(0.1)
        else:
            logs = command("journalctl", "-u", unit, "--no-pager", "-n", "20")
            raise AssertionError(f"isolated user manager did not start: {last_error}\n{logs.stdout}")
        yield UserManager(environment, manager_pid(runtime))
    finally:
        try:
            retire()
        finally:
            stopped = command("systemctl", "stop", unit)
            assert stopped.returncode == 0, stopped.stdout + stopped.stderr
            state = command("systemctl", "show", unit, "--property=ActiveState", "--value")
            assert state.returncode == 4 or state.stdout.strip() == "inactive", state.stdout + state.stderr
