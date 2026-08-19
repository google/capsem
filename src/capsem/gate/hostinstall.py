"""Build this checkout and put it on the machine you are sitting at.

`just install` did this until `2c2f977c refactor: lock disciplined public
surfaces` removed every recipe carrying a shell body. The capability went with
the body: nothing since builds the local tree and installs it, so a developer
wanting to run what they just wrote had to install a published release or read a
deleted recipe out of the history.

It comes back as a plan rather than as the ninety lines of bash it was, and it
builds nothing. `just test` already produces the exact release-mode package this
project ships and `[prefix] exports` carries it back into `dist/`, so this
installs that. A second way to build one would be a second thing to keep true,
and it would not be the package that was just qualified.

What it adds is the three steps the gate does not already do, because the gate's
own install proof happens in a container and this one happens to you:

  stop what is running, or macOS respawns the service mid-install and Linux
  writes over binaries that are open

  install the package the rail just built

  wait for the service to answer, which is the only evidence that means anything

The isolation variables are cleared first and that is not hygiene. A shell that
has been running `just test` exports `CAPSEM_HOME` at `target/test-home`, and
installing with it set bakes that path into the systemd unit or LaunchAgent --
which the next test run wipes, leaving a service pointing at nothing.
"""

from __future__ import annotations

import getpass
import os
import subprocess
import time
from pathlib import Path

from . import host
from .actions import Call
from .command import GateCommand
from .config import GateConfig
from .context import Context
from .errors import GateError
from .execution import Kind, Needs, Speed, Step, step
from .opacity import CallJustification, Effect, OpaqueKind, machine_effects
from .plan import Plan
from .testmodules import InWorkspace
from .versions import workspace_version


class HostInstallModule(
    InWorkspace,
    GateCommand,
    name="install-host",
    help="build this checkout's package and install it on this machine",
):
    """What `just install` dispatches to."""

    def plan(self) -> Plan:
        plan = Plan(self.name)
        host_install(plan, self._config)
        return plan


def host_install(plan: Plan, config: GateConfig, *, after: tuple[Step, ...] = ()) -> Step:
    """Stop, install what `just test` built, and wait for it to answer."""
    phase = plan.phase("install-host")
    stopped = phase.add(
        step(
            "stop",
            Call(
                "stop the running service so the package does not land underneath it",
                _stop,
                justification=CallJustification(
                    kind=OpaqueKind.DOMAIN_TRANSACTION,
                    reason="signalling this machine's own processes is not a command a plan can order",
                    effects=machine_effects(Effect.PROCESS, Effect.HOST_STATE),
                ),
            ),
            kind=Kind.E2E,
            needs=frozenset({Needs.DISK}),
            speed=Speed.FAST,
        ),
        after=after,
    )
    installed = phase.add(
        step(
            "package",
            Call(
                "install the package the gate exported",
                _install,
                justification=CallJustification(
                    kind=OpaqueKind.DOMAIN_TRANSACTION,
                    reason="dpkg decides between install and dependency repair from its own exit",
                    effects=machine_effects(Effect.PROCESS, Effect.FILESYSTEM, Effect.HOST_STATE),
                ),
            ),
            kind=Kind.E2E,
            needs=frozenset({Needs.DISK}),
            speed=Speed.SLOW,
        ),
        after=(stopped,),
    )
    return phase.add(
        step(
            "health",
            Call(
                "wait for the installed service to answer",
                _healthy,
                justification=CallJustification(
                    kind=OpaqueKind.PURE_INSPECTION,
                    reason="an installed service answering is the only evidence that it installed",
                    effects=machine_effects(Effect.PROCESS),
                ),
            ),
            kind=Kind.E2E,
            needs=frozenset({Needs.DISK}),
            speed=Speed.FAST,
        ),
        after=(installed,),
    )


def _settings(context: Context):
    return context.config.install.host


def _stop(context: Context) -> None:
    host = _settings(context)
    plist = Path.home() / host.macos_agent_plist
    if plist.is_file():
        for argv in (
            ["launchctl", "bootout", host.macos_launch_domain.format(uid=os.getuid()), str(plist)],
            ["launchctl", "unload", str(plist)],
        ):
            if context.runner.run(argv, check=False) == 0:
                break
    for name in host.processes:
        context.runner.run(["pkill", "-9", "-x", name], check=False)
    time.sleep(0.5)
    run_dir = Path.home() / host.run_dir
    for leftover in host.run_files:
        (run_dir / leftover).unlink(missing_ok=True)


def _install(context: Context) -> None:
    """Each platform's own package, installed the way that platform installs.

    Not one path with a branch bolted on: macOS ships a `.pkg` whose postinstall
    registers a LaunchAgent, and Linux ships a `.deb` whose dependencies may need
    repairing. They share the sentence "install what the gate built" and nothing
    below it.
    """
    if host.on_macos():
        _install_macos(context)
        return
    _install_linux(context)


def _install_linux(context: Context) -> None:
    dist = context.config.path(context.config.package.dist_dir)
    suffix = f"_{context.config.host_arch().dpkg}{context.config.package.package_suffix}"
    built = sorted(path for path in dist.glob("*") if path.name.endswith(suffix))
    if not built:
        raise GateError(
            f"no {suffix} package in {dist}: this installs what `just test` "
            "built and exported, so run that first."
        )
    # `-f -y` only when dpkg refused: it repairs dependencies, and running it
    # unconditionally would let a clean install look like one that needed repair.
    if context.runner.run(["sudo", "dpkg", "-i", str(built[-1])], check=False) != 0:
        context.runner.run(["sudo", "apt-get", "install", "-f", "-y"])


def _install_macos(context: Context) -> None:
    settings = _settings(context)
    version = workspace_version(context.config.root)
    package = context.config.path(settings.macos_package.format(version=version))
    if not package.is_file():
        raise GateError(
            f"no package at {package}: this installs what `just test` built and "
            "exported, so run that first."
        )
    request = context.config.path(settings.macos_user_request_script)
    # Written before and cleared after, including on failure: the postinstall
    # reads it to decide whose LaunchAgent to register, and a request left
    # behind would answer for whoever installs next.
    context.runner.run(["bash", str(request), "write", getpass.getuser()])
    try:
        argv = ["installer", "-pkg", str(package), "-target", "/"]
        context.runner.run(argv if os.getuid() == 0 else ["sudo", *argv])
    finally:
        context.runner.run(["bash", str(request), "clear"], check=False)


def _healthy(context: Context) -> None:
    host = _settings(context)
    socket = Path.home() / host.run_dir / host.service_socket
    deadline = time.monotonic() + host.health_seconds
    while time.monotonic() < deadline:
        if socket.is_socket():
            probe = subprocess.run(
                ["curl", "-s", "--unix-socket", str(socket), "--max-time", "2", host.health_url],
                capture_output=True,
                check=False,
            )
            if probe.returncode == 0:
                return
        time.sleep(0.5)
    raise GateError(
        f"the package installed but its service did not answer within "
        f"{host.health_seconds:.0f}s on {socket}. Check `capsem status` and the "
        "service log before trusting this install."
    )
