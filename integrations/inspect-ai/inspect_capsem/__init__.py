"""Standalone Capsem SandboxEnvironment extension for Inspect AI."""

from __future__ import annotations

from inspect_capsem._controller import (
    CapsemController,
    CommandResult,
    SdkCapsemController,
)
from inspect_capsem._registry import capsem_sandbox_environment
from inspect_capsem._tools import (
    INSPECT_SANDBOX_TOOLS_GUEST_DIR,
    INSPECT_SANDBOX_TOOLS_GUEST_PATH,
    bake_sandbox_tools_into_controller,
    resolve_inspect_sandbox_tools_host_binary,
)
from inspect_capsem.config import CapsemSandboxConfig
from inspect_capsem.sandbox import CapsemSandboxEnvironment

__all__ = [
    "INSPECT_SANDBOX_TOOLS_GUEST_DIR",
    "INSPECT_SANDBOX_TOOLS_GUEST_PATH",
    "CapsemController",
    "CapsemSandboxConfig",
    "CapsemSandboxEnvironment",
    "CommandResult",
    "SdkCapsemController",
    "bake_sandbox_tools_into_controller",
    "capsem_sandbox_environment",
    "resolve_inspect_sandbox_tools_host_binary",
]
