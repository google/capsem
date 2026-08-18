"""Environment policy at command and action boundaries."""

from __future__ import annotations

from collections.abc import Mapping

from .config import GateConfig
from .sandbox import SandboxMode


def command_environment(
    config: GateConfig,
    inherited: Mapping[str, str],
    mode: SandboxMode,
    *,
    source_commit: str | None = None,
) -> dict[str, str]:
    """Add the owning command's typed sandbox policy to its normal scope.

    And which commit the run is proving, when the gate could establish one. A
    step that authors release provenance has to be *told* that rather than
    resolve it: the tree a script sits in is not always the subject -- inside
    the install container it is a mount -- and the gate is the only party that
    knows. Absent when there is no commit to name, which is a checkout with no
    history, and no step may then quietly invent one.
    """
    scoped = {**inherited, config.environment.command_sandbox_mode: mode.value}
    if source_commit is not None:
        scoped[config.environment.qualified_source_commit] = source_commit
    return scoped


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
