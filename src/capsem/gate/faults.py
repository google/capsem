"""What a filesystem fault is made of, and how to find it out cheaply.

Separated from the watcher because the rules are worth testing against a list
of facts, without a disk, a scheduler, or a sixty-minute gate behind them.
"""

from __future__ import annotations

import hashlib
import stat
import subprocess
from dataclasses import dataclass
from pathlib import Path

#: Directories under the checkout a run may write. Everything else is input:
#: the gate reads it, and changing it mid-run means the thing being qualified
#: is not the thing that was measured.
BUILD_OUTPUT = frozenset({"target", ".git", "node_modules", ".venv"})

#: Hash files up to this size. Digests answer "are these the same bytes under
#: two names", which matters for seeds and manifests; a multi-gigabyte rootfs
#: is answered by inode and size at a fraction of the cost.
DIGEST_LIMIT = 1 << 20


@dataclass(frozen=True, slots=True)
class Facts:
    """Everything one `stat` already paid for, plus a digest when it is cheap.

    Typed rather than a dict: these reach `Event` directly, and a loose
    `dict[str, int | str | None]` made `ty` unable to tell a mode from a
    digest -- which is the sort of thing that reads fine and stores a hash in
    a permission field.
    """

    mode: int | None = None
    size: int | None = None
    inode: int | None = None
    links: int | None = None
    digest: str | None = None


@dataclass(frozen=True, slots=True)
class Event:
    """One observed change and who caused it."""

    at: float
    kind: str
    path: Path
    steps: tuple[str, ...]
    facts: Facts = Facts()

    @property
    def mode(self) -> int | None:
        return self.facts.mode

    @property
    def inode(self) -> int | None:
        return self.facts.inode

    @property
    def links(self) -> int | None:
        return self.facts.links

    @property
    def digest(self) -> str | None:
        return self.facts.digest


@dataclass(frozen=True, slots=True)
class Fault:
    """A rule broken, in the terms someone can act on."""

    path: Path
    steps: tuple[str, ...]
    reason: str
    detail: str

    def render(self) -> str:
        who = ", ".join(self.steps) or "no step in flight"
        return f"[{self.reason}] {self.path}: {self.detail} (steps: {who})"


def facts_of(path: Path) -> Facts:
    """One `stat`, and a digest only when the file is small enough to be worth it."""
    try:
        info = path.stat()
    except OSError:
        return Facts()
    digest = None
    if stat.S_ISREG(info.st_mode) and 0 < info.st_size <= DIGEST_LIMIT:
        try:
            digest = hashlib.blake2b(path.read_bytes(), digest_size=16).hexdigest()
        except OSError:
            digest = None
    return Facts(
        mode=stat.S_IMODE(info.st_mode),
        size=info.st_size,
        inode=info.st_ino,
        links=info.st_nlink,
        digest=digest,
    )


def source_inodes(source_root: Path) -> dict[int, Path]:
    """Every checked-in file, keyed by inode.

    A hardlink into build output creates a directory entry in `target/` and
    leaves the source directory untouched, so watching the source tree cannot
    see it happen: the only trace is that the new file's inode is one of
    these. `git ls-files` rather than a walk -- it is the authority on what is
    tracked, it does not descend into build output, and it costs milliseconds.
    """
    try:
        listing = subprocess.run(
            ["git", "ls-files", "-z"],
            cwd=source_root,
            capture_output=True,
            text=True,
            check=True,
        ).stdout
    except (OSError, subprocess.CalledProcessError):
        return {}

    inodes: dict[int, Path] = {}
    for name in listing.split("\0"):
        if not name:
            continue
        path = source_root / name
        try:
            inodes[path.stat().st_ino] = path
        except OSError:
            continue
    return inodes
