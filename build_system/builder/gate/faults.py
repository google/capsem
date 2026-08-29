"""What a filesystem fault is made of, and how to find it out cheaply.

Separated from the watcher because the rules are worth testing against a list
of facts, without a disk, a scheduler, or a sixty-minute gate behind them.
"""

from __future__ import annotations

import hashlib
import stat
import subprocess
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from typing import Literal

#: Directories under the checkout a run may write. Everything else is input:
#: the gate reads it, and changing it mid-run means the thing being qualified
#: is not the thing that was measured.
#:
#: `dist`, `packages` and `assets` are here because they are gitignored build
#: roots the gate rewrites every run -- `target/assets/current` is resynced per
#: architecture, and stale `.deb`s are removed before each package build. With
#: only `target` excluded, ordinary steps read as the gate mutating the tree it
#: is qualifying.
BUILD_OUTPUT = frozenset({"target", "dist", "packages", "assets", ".git", "node_modules", ".venv"})


def ignored_here(root: Path, directory: Path) -> bool:
    """Whether git ignores `directory`, asked once per directory and cached.

    Not a snapshot. The first version listed every ignored path at run start
    with `git ls-files --ignored`, which reports only paths that *exist* -- and
    the case that matters most is a tree the run creates: `crates/capsem-app/
    gen/` is gitignored, so it is not in the private copy at all until Tauri's
    build script makes it, and every file under it was still reported as source.
    Asking about the rules rather than about today's files is the difference.

    Memoized per directory because `is_source` is on the path of every
    filesystem operation the gate performs, while the number of distinct
    directories it touches is small. `git check-ignore` answers about a path
    whether or not it exists, which is exactly the property needed.
    """
    import subprocess

    if not root.is_dir():
        return False
    cache = _IGNORED.setdefault(root, {})
    key = str(directory)
    if key not in cache:
        cache[key] = (
            subprocess.run(
                ["git", "check-ignore", "-q", "--no-index", key],
                cwd=root,
                check=False,
                capture_output=True,
            ).returncode
            == 0
        )
    return cache[key]


def duplication_expected(path: Path, root: Path, exempt: tuple[str, ...]) -> bool:
    """Whether identical bytes under this path are a third party's doing.

    Beside `is_source` because it answers the same kind of question about a
    path. An exact list of trees, never a blanket exemption for generated
    files -- the duplicate-content rule earns its place in build output too,
    where it caught a lane copying a hardlinked alias tree into distinct
    inodes. `[runlog] duplicate_content_exempt` carries the list and the why.
    """
    if not exempt:
        return False
    try:
        relative = path.resolve().relative_to(root)
    except (ValueError, OSError):
        return False
    # Component-wise, so `crates/capsem-app/gen` cannot also match
    # `crates/capsem-app/generated-elsewhere`.
    return any(
        relative.parts[: len(PurePosixPath(entry).parts)] == PurePosixPath(entry).parts
        for entry in exempt
    )


def is_source(path: Path, root: Path) -> bool:
    """Whether an absolute path is checked-in input rather than build output."""
    # A relative path can come from an operation using dir_fd. Resolving it
    # against the gate's cwd would falsely blame an unrelated source path.
    if not path.is_absolute():
        return False
    try:
        relative = path.resolve().relative_to(root)
    except (ValueError, OSError):
        return False
    if not relative.parts or relative.parts[0] in BUILD_OUTPUT:
        return False
    # Ask git about rules, not the paths present at run start: generated,
    # ignored trees often do not exist until a build creates them.
    return not ignored_here(root, relative.parent)


#: Per checkout, per directory. A run observes one tree; a test may build
#: several, so this is keyed by root rather than global.
_IGNORED: dict[Path, dict[str, bool]] = {}

Attribution = Literal["exact", "candidates"]


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
    """One observed change and its exact writer or live-step candidates."""

    at: float
    kind: str
    path: Path
    steps: tuple[str, ...]
    facts: Facts = Facts()
    attribution: Attribution = "exact"
    """Whether ``steps`` are writers or merely the steps live at notification."""

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
