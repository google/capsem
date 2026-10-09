"""Standalone Capsem SandboxEnvironment extension for Inspect AI."""

from __future__ import annotations

from typing import Any

from inspect_capsem.config import CapsemSandboxConfig
from inspect_capsem.sandbox import CapsemSandboxEnvironment

__all__ = [
    "CapsemSandboxConfig",
    "CapsemSandboxEnvironment",
]


def __getattr__(name: str) -> Any:
    if name == "HostBuildGrant":
        from inspect_capsem.containers.build_grant import HostBuildGrant

        return HostBuildGrant
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")
