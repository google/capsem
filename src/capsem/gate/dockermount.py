"""Where a container sees the host, and what it is allowed to see.

Split from `docker`, which owns the operations. This owns the *addressing*:
what may be mounted at all, and where a checkout path appears once it is. The
two change for different reasons -- a new container operation is routine, and a
new mount of the working tree is the race that killed a release run.
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from .errors import GateError


@dataclass(frozen=True)
class Mount:
    """A bind mount or named volume, in `-v` order.

    Refuses the checkout. `-v <repo_root>:/src` let a host step churning
    hardlinks and a container reading the same inodes over virtiofs share a
    filesystem neither declared, which killed a release run with an
    intermittent `Permission denied` on a file that was `0644` before and
    after. Containers get their source copied into an image instead, so there
    is no mount to police.
    """

    source: str
    target: str
    options: str = ""

    #: Set only by `unmigrated`. A mount of the checkout is a defect; until the
    #: four modules that still do it are converted to `COPY`, each one says so
    #: at its call site rather than the guard being switched off globally.
    legacy: bool = False

    @classmethod
    def unmigrated(cls, source: str, target: str, options: str = "") -> Mount:
        """A checkout mount that has not been converted to an image copy yet.

        Deliberately ugly and deliberately greppable. `tests/
        test_gate_docker_boundary.py` counts these and refuses a new one, so
        the list can only shrink.
        """
        return cls(source=source, target=target, options=options, legacy=True)

    def __post_init__(self) -> None:
        if self.legacy:
            return
        # The checkout this package was imported from -- the same derivation
        # `sourcestate.gate_source()` uses, because a `Mount` is constructed
        # before any config is in hand and asking for one would put the check
        # back at the call sites it exists to remove.
        root = Path(__file__).resolve().parents[3]
        # Docker's own rule: a source with no separator is a *named volume*,
        # not a path. Resolving one relative to the cwd puts it inside the
        # checkout and refuses every legitimate cache volume.
        if "/" not in self.source:
            return
        try:
            candidate = Path(self.source).resolve()
        except (OSError, ValueError):
            return
        if candidate == root or root in candidate.parents:
            raise GateError(
                f"{self.source} is inside the checkout: a container that mounts the "
                "working tree shares inodes with every host step, which is a race "
                "no declaration can constrain. COPY the source into the image."
            )

    def __str__(self) -> str:
        return f"{self.source}:{self.target}" + (f":{self.options}" if self.options else "")


def container_path(root: Path, host_path: Path, *, mount: str) -> str:
    """Where a checkout path appears inside a container that bind-mounts it."""
    host_path = Path(host_path)
    root = Path(root)
    try:
        relative = host_path.relative_to(root)
    except ValueError:
        raise GateError(f"{host_path} is outside the mounted checkout {root}") from None
    return f"{mount}/{relative}"
