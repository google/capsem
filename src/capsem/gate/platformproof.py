"""Proving the package runs where `[platforms]` says it does.

Both callers are here because they ask the same question at different moments:
the release lane plans it as a step against the pulled package, and the local
install transaction runs it against the package it just built.

Neither could be left to the install proof itself. That runs inside
`docker/Dockerfile.install-test`, whose base is `ubuntu:24.04` -- the same
glibc the binaries are built against -- so a wrong platform floor is invisible
to it by construction. The 0.6.0 package declared no libc dependency at all,
passed the whole suite, and then failed on every user below glibc 2.39.
"""

from __future__ import annotations

from .actions import Script
from .config import GateConfig
from .errors import GateError
from .execution import Kind, Needs, Speed, Step, step


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
        Script(config.modules.platform_support_script, "--package", package),
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
