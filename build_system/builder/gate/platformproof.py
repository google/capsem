"""Proving the package runs where `[platforms]` says it does.

Both callers are here because they ask the same question at different moments:
the release lane plans it as a step against the pulled package, and the local
install transaction runs it against the package it just built.

Neither could be left to the install proof itself. That runs inside
`build_system/docker/Dockerfile.install-test`, whose base is `ubuntu:24.04` -- the same
glibc the binaries are built against -- so a wrong platform floor is invisible
to it by construction. The 0.6.0 package declared no libc dependency at all,
passed the whole suite, and then failed on every user below glibc 2.39.
"""

from __future__ import annotations

from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path

from capsem_builder import crypto

from . import cancellation
from . import config as gate_config
from .actions import Script
from .config import GateConfig
from .docker import Docker
from .errors import GateError
from .execution import Kind, Needs, Speed, Step, step
from .lifecycle import Resource, held
from .proc import Runner


class _ProbeContainer(Resource, name="platform-probe-container"):
    """Reserve one nonce-owned name BEFORE the Docker client can create it.

    No predecessor/stale-name removal: another invocation has another UUID.
    Release remains strict even if creation/attachment was interrupted, so
    ambiguous daemon settlement is a failure, never inferred absence.
    """

    def __init__(self, docker: Docker, prefix: str) -> None:
        self._docker = docker
        self.container = f"{prefix}-platform-{crypto.uuid_v4().hex}"

    def acquire(self) -> None:
        pass  # The unique name is the owned slot; creation occurs inside held.

    def release(self) -> None:
        self._docker.remove(self.container, strict=True)


@contextmanager
def probe_container(repository: Path) -> Iterator[str]:
    """Use the gate's existing signal unwinding and exact Docker settlement."""
    config = gate_config.for_root(repository)
    container = _ProbeContainer(Docker(Runner(repository)), config.install.container)
    with cancellation.unwind_sigterm(), held(container):
        yield container.container


def platform_step(config: GateConfig, package: str | None) -> Step:
    """The release lane's step: prove the exact package about to be published.

    Only a pulled lane has an exact package, and only a pulled lane plans this
    -- but `pulled` is a property, so nothing narrows the field for a reader or
    a type checker. Say it rather than assume it: a proof about a package
    cannot be planned without one.
    """
    if package is None:
        raise GateError("the platform support proof needs the exact package it proves")
    return step(
        "platform-support",
        Script(config, config.modules.platform_support_script, "--package", package),
        contends=(config.exclusive("docker_daemon"),),
        kind=Kind.E2E,
        needs=frozenset({Needs.DOCKER, Needs.DISK}),
        speed=Speed.SLOW,
    )


def prove(runner, config: GateConfig, package) -> None:
    """The local lane's call, before the install proof that cannot see this."""
    runner.step("Proving declared platform support")
    runner.run(
        [
            "python3",
            str(runner.root / config.modules.platform_support_script),
            "--package",
            str(package),
        ]
    )
