"""Shared Capsem micro-VM controller, file transfer, exec, and OCI container helpers."""

from __future__ import annotations

from typing import Any

from capsem_integrations._controller import (
    CapsemController,
    CommandResult,
    SdkCapsemController,
    truncate_label_value,
)

__all__ = [
    "CapsemController",
    "CommandResult",
    "HostBuildGrant",
    "SdkCapsemController",
    "default_host_build_policy",
    "truncate_label_value",
]


def __getattr__(name: str) -> Any:
    if name in ("HostBuildGrant", "default_host_build_policy"):
        from capsem_integrations.containers.build_grant import (
            HostBuildGrant,
            default_host_build_policy,
        )

        return HostBuildGrant if name == "HostBuildGrant" else default_host_build_policy
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")
