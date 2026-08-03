"""Docker mount required to preserve Git identity from a linked worktree."""

from __future__ import annotations

from pathlib import Path

from .errors import GateError
from .proc import Runner


def docker_git_metadata_mount(runner: Runner) -> tuple[str, ...]:
    """Mount external worktree metadata at the absolute path in ``.git``.

    An ordinary checkout carries its ``.git`` directory inside the source
    mount and needs nothing extra. A linked worktree instead carries a file
    whose ``gitdir:`` target lives under the primary checkout. Docker cannot
    follow that host-only path unless the common metadata directory is mounted
    at the same absolute location inside the container.
    """
    if not (runner.root / ".git").is_file():
        return ()

    common = runner.capture(
        ["git", "rev-parse", "--path-format=absolute", "--git-common-dir"],
        check=False,
    )
    common_dir = Path(common)
    if not common or not common_dir.is_absolute() or not common_dir.is_dir():
        raise GateError(
            "linked worktree Git metadata could not be resolved; refusing a "
            "Docker build that would embed an unknown source revision"
        )
    return ("-v", f"{common_dir}:{common_dir}:ro")
