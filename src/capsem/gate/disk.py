"""What the gate is allowed to occupy, and how it gives it back.

A run builds two architectures of VM images, a package cohort, a release
channel, an install container and its assets. None of that is small, and until
now nothing bounded it: a crashed run reclaimed nothing, and the next one
started with less room than the last.

Reclaiming is policy rather than whoever remembers a path. `[disk] reclaimable`
lists every tree the gate can create; nothing outside it may be removed, and
the loader already refuses an entry that is absolute or escapes upwards. This
module adds the second half of that guarantee: a symlink inside a reclaimable
tree is unlinked, never followed, so a link someone left pointing at their home
directory takes the link with it and nothing else.

`ensure_space` reclaims first and only then refuses. Failing at minute thirty
of a forty-minute run, having deleted nothing, is the worst of both.
"""

from __future__ import annotations

import shutil
from dataclasses import dataclass
from pathlib import Path

from .config import GateConfig
from .errors import GateError
from .runhistory import free_gb, tree_size


@dataclass(frozen=True)
class Reclaimed:
    """What a reclaim actually recovered."""

    trees: dict[str, int]
    free_before_gb: float
    free_after_gb: float

    @property
    def bytes_freed(self) -> int:
        return sum(self.trees.values())

    @property
    def gb_freed(self) -> float:
        return self.bytes_freed / 1024**3


def roots(config: GateConfig) -> list[Path]:
    """Every reclaimable tree that exists right now, resolved to this checkout."""
    return [
        config.path(relative)
        for relative in config.disk.reclaimable
        if config.path(relative).exists()
    ]


def footprint(config: GateConfig) -> dict[str, int]:
    """What each reclaimable tree currently occupies."""
    return {
        relative: tree_size(config.path(relative))
        for relative in config.disk.reclaimable
        if config.path(relative).is_dir()
    }


def reclaim(config: GateConfig, *, keep: tuple[str, ...] = ()) -> Reclaimed:
    """Remove the reclaimable trees, and report what that recovered.

    `keep` names trees this run still needs -- its own run log, most obviously.
    Named rather than inferred, so a caller that needs one has to say so.
    """
    before = free_gb(config.root)
    freed: dict[str, int] = {}

    for relative in config.disk.reclaimable:
        if relative in keep:
            continue
        target = config.path(relative)
        if not target.is_dir():
            continue
        freed[relative] = tree_size(target)
        _remove_tree(target, config.root)

    return Reclaimed(freed, before, free_gb(config.root))


def ensure_space(config: GateConfig, phase: str) -> Reclaimed:
    """Make room for an expensive phase, or refuse it before it starts.

    Refusing early is the point. Discovering there is no disk an hour into a
    VM asset build wastes the hour, and leaves a half-built tree that the next
    run has to reclaim before it can begin.
    """
    required = config.disk.required_free_gb
    if free_gb(config.root) >= required:
        return Reclaimed({}, free_gb(config.root), free_gb(config.root))

    recovered = reclaim(config, keep=(config.runlog.root,))
    if recovered.free_after_gb >= required:
        return recovered

    raise GateError(
        f"{phase} needs {required}GB free and there is "
        f"{recovered.free_after_gb:.1f}GB after reclaiming "
        f"{recovered.gb_freed:.1f}GB. Free space outside the checkout, or run "
        f"`capsem-gate gc --aggressive` to release the Docker rails too."
    )


def _remove_tree(target: Path, root: Path) -> None:
    """Delete a tree, refusing anything that is not inside the checkout.

    Belt and braces over the loader's validation: that checks the configured
    strings, and this checks the resolved path. A reclaimable entry that turns
    out to be a symlink to somewhere else is unlinked rather than followed --
    the alternative is deleting whatever it pointed at.
    """
    if target.is_symlink():
        target.unlink()
        return

    resolved = target.resolve()
    if resolved != root.resolve() and root.resolve() not in resolved.parents:
        raise GateError(f"refusing to reclaim {resolved}: it resolves outside {root}")

    # `rmtree` unlinks the symlinks it meets rather than following them, which
    # is the behaviour the test above pins; the guard it does not have is the
    # one above, for a root that is itself a link somewhere else.
    shutil.rmtree(target)
