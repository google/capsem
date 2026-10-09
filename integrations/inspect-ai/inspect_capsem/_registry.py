"""Inspect AI @sandboxenv entry point registration for Capsem."""

from __future__ import annotations

from inspect_ai.util import sandboxenv

from inspect_capsem.sandbox import CapsemSandboxEnvironment


@sandboxenv(name="capsem")
def capsem_sandbox_environment() -> type[CapsemSandboxEnvironment]:
    """Return the CapsemSandboxEnvironment class registered under 'capsem'."""
    return CapsemSandboxEnvironment
