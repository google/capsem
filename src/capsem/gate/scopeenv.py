"""Environment policy at command and action boundaries."""

from __future__ import annotations

from collections.abc import Mapping

from .config import GateConfig
from .sandbox import SandboxMode


def command_environment(
    config: GateConfig, inherited: Mapping[str, str], mode: SandboxMode
) -> dict[str, str]:
    """Add the owning command's typed sandbox policy to its normal scope."""
    return {**inherited, config.environment.command_sandbox_mode: mode.value}


def action_environment(
    config: GateConfig,
    inherited: Mapping[str, str],
    own: Mapping[str, str],
    *,
    outside_sandbox: bool,
) -> dict[str, str]:
    """Merge an action's environment, clearing command policy outside it."""
    merged = {**inherited, **own}
    if outside_sandbox:
        # Runner overlays onto os.environ, so omission would reveal any forged
        # ambient value. Empty is what shell `${NAME:-}` treats as absent.
        merged[config.environment.command_sandbox_mode] = ""
    return merged
