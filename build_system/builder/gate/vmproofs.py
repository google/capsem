"""The two VM drivers that are not pytest, and where they point.

`injection_test.py` and `integration_test.py` predate the pytest suites and
still own proofs nothing else makes. Both take the same three coordinates --
the binary, the assets, the materialized profiles -- and both were spelled out
at four call sites each, once per profile in the matrix.

Each coordinate has an environment override, because a release lane runs the
same proof against pulled artifacts rather than source-built ones. Resolved
here, once, rather than by four `${VAR:-default}` expansions that agreed by
convention.
"""

from __future__ import annotations

import os
from pathlib import Path

from .actions import Script
from .config import GateConfig
from .execution import Kind, Needs, Speed, Step, step


def _binary(config: GateConfig) -> str:
    settings = config.functional
    return os.environ.get(settings.binary_variable, settings.binary)


def _assets(config: GateConfig) -> str:
    settings = config.functional
    return os.environ.get(settings.assets_variable, settings.assets_dir)


def _profiles_dir(config: GateConfig) -> str:
    settings = config.functional
    root = os.environ.get(settings.config_root_variable, settings.config_root)
    return str(Path(root) / settings.profiles_subdir)


def injection(
    config: GateConfig,
    *,
    profile: str,
    assets: str | None = None,
    profiles_dir: str | None = None,
) -> Step:
    """Prove the guest refuses what it is supposed to refuse."""
    settings = config.functional
    return step(
        f"injection.{profile}",
        Script(
            config,
            settings.injection_script,
            "--binary",
            _binary(config),
            "--assets",
            assets or _assets(config),
            "--profiles-dir",
            profiles_dir or _profiles_dir(config),
            "--profile",
            profile,
        ),
        contends=(config.exclusive("apple_vz"),),
        kind=Kind.CAPSEM,
        needs=frozenset({Needs.VM, Needs.KVM, Needs.DISK}),
        speed=Speed.SLOW,
    )


def integration(
    config: GateConfig,
    *,
    profile: str,
    assets: str | None = None,
    profiles_dir: str | None = None,
) -> Step:
    """Boot a real VM and drive it the way a user would."""
    settings = config.functional
    return step(
        f"integration.{profile}",
        Script(
            config,
            settings.integration_script,
            "--binary",
            _binary(config),
            "--assets",
            assets or _assets(config),
            "--profile",
            profile,
            env=config.environment.content(profiles=profiles_dir or _profiles_dir(config)),
        ),
        contends=(config.exclusive("apple_vz"),),
        kind=Kind.CAPSEM,
        needs=frozenset({Needs.VM, Needs.KVM, Needs.DISK}),
        speed=Speed.SLOW,
    )
