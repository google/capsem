"""The two VM drivers that are not pytest, and where they point.

`injection_test.py` and `integration_test.py` predate the pytest suites and
still own proofs nothing else makes. Both take the same two coordinates --
the binary and the assets -- and both were spelled out at four call sites
each before this module named them once.

Each coordinate has an environment override, because a release lane runs the
same proof against pulled artifacts rather than source-built ones. Resolved
here, once, rather than by four `${VAR:-default}` expansions that agreed by
convention.
"""

from __future__ import annotations

import os

from . import pytestsuite
from .actions import Script
from .config import GateConfig
from .execution import Kind, Needs, Speed, Step, step


def _binary(config: GateConfig) -> str:
    settings = config.functional
    return os.environ.get(settings.binary_variable, settings.binary)


def _assets(config: GateConfig) -> str:
    settings = config.functional
    return os.environ.get(settings.assets_variable, settings.assets_dir)


def injection(
    config: GateConfig,
    *,
    assets: str | None = None,
) -> Step:
    """Prove the guest refuses what it is supposed to refuse."""
    settings = config.functional
    return step(
        "injection",
        Script(
            config,
            settings.injection_script,
            "--binary",
            _binary(config),
            "--assets",
            assets or _assets(config),
        ),
        contends=pytestsuite.sharing(config),
        kind=Kind.CAPSEM,
        needs=frozenset({Needs.VM, Needs.KVM, Needs.DISK}),
        speed=Speed.SLOW,
    )


def integration(
    config: GateConfig,
    *,
    assets: str | None = None,
) -> Step:
    """Boot a real VM and drive it the way a user would."""
    settings = config.functional
    return step(
        "integration",
        Script(
            config,
            settings.integration_script,
            "--binary",
            _binary(config),
            "--assets",
            assets or _assets(config),
        ),
        contends=pytestsuite.sharing(config),
        kind=Kind.CAPSEM,
        needs=frozenset({Needs.VM, Needs.KVM, Needs.DISK}),
        speed=Speed.SLOW,
    )
