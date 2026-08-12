"""Defer the warm asset shortcut until the invariant graph executes."""

from __future__ import annotations

from threading import Lock

from .actions import Action
from .config import Arch, GateConfig
from .context import Context


def missing(config: GateConfig, arch: Arch) -> list[str]:
    """Required boot artifacts that are absent or truncated."""
    tree = config.path(config.imagebuild.output) / arch.name
    return [
        name
        for name in config.artifacts.bootable
        if not (tree / name).is_file() or (tree / name).stat().st_size == 0
    ]


class AssetRecovery:
    """One thread-safe warm/cold decision shared by a recovery cohort."""

    def __init__(self, config: GateConfig, arch: Arch) -> None:
        self._config = config
        self._arch = arch
        self._needed: bool | None = None
        self._lock = Lock()

    def needed(self) -> bool:
        with self._lock:
            if self._needed is None:
                self._needed = bool(missing(self._config, self._arch))
            return self._needed

    def when(self, action: Action) -> WhenAssetsMissing:
        return WhenAssetsMissing(self, action)


class WhenAssetsMissing(Action, name="when-assets-missing"):
    """Run an action only when the host asset cohort needs recovery.

    The predicate deliberately lives in ``perform``. Evaluating it in a plan
    constructor made the public checkout and its private prefix describe
    different labels, so a private failure could not be selected by ``--from``.
    """

    def __init__(self, recovery: AssetRecovery, action: Action) -> None:
        self._recovery = recovery
        self._action = action

    def render(self) -> str:
        return f"when host assets are missing: {self._action.render()}"

    def perform(self, context: Context) -> None:
        if self._recovery.needed():
            self._action.perform(context)
