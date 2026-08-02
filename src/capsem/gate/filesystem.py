"""How the gate touches the filesystem, as ordinary functions.

Split from `fileactions`, which wraps these same operations as plan actions.
The seam is whether the caller is composing a step or already inside one: a
phase whose argv cannot be rendered at plan time -- a package build carries
signing material -- still must not reach for raw `Path.mkdir`, and these are
what it reaches for instead.

Every one of them exists because a call site got it wrong once: removal that
reported success on an undeletable path, a copy that replaced the sibling it
should have merged with, a temp directory in `$TMPDIR` that blew the socket
path limit, and teardown that raised over the failure it was cleaning up
after.
"""

from __future__ import annotations

import contextlib
import hashlib
import shutil
import tempfile
from pathlib import Path

import blake3

from .errors import GateError

#: Digest algorithms the gate can record, by the name configuration uses.
DIGESTS = ("blake3", "sha256")


def digest_of(path: Path, *, algorithm: str) -> str:
    """Hash a file, in whichever algorithm configuration names.

    Streamed rather than read whole: a rootfs is measured in gigabytes, and a
    gate that needs the artifact in memory to describe it is a gate that fails
    on the machine building the largest one.
    """
    if algorithm not in DIGESTS:
        raise GateError(f"unknown digest {algorithm!r}; expected one of {', '.join(DIGESTS)}")

    hasher = blake3.blake3() if algorithm == "blake3" else hashlib.sha256()

    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def write_text(path: Path, text: str) -> None:
    """Write a file the gate produced, creating its directory.

    A helper rather than an `Action`, for the same reason `digest_of` is one:
    the value is only known while a step is running. The revision under test is
    *captured*, not declared, so an action would have to be constructed with
    something that does not exist yet.
    """
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def make_dir(path: Path) -> None:
    """Create a directory and its parents, tolerating one already there.

    The function form of `MakeDir`, for the phases whose argv cannot be
    rendered at plan time -- a package build's environment carries signing
    material, so the action that runs it must not be printed by `--dry-run`.
    Going through here keeps those phases off raw `Path.mkdir` all the same.
    """
    path.mkdir(parents=True, exist_ok=True)


def copy_tree(source: Path, target: Path) -> None:
    """Copy a directory, replacing whatever was at the target."""
    remove(target)
    shutil.copytree(source, target)


def merge_tree(source: Path, target: Path) -> None:
    """Copy a directory *into* an existing one, keeping what is already there.

    Distinct from `copy_tree`, which replaces. Merging is what an asset lane
    does when several architectures land in one tree, and doing it by replacing
    would delete the sibling that arrived first.
    """
    shutil.copytree(source, target, dirs_exist_ok=True)


def link(path: Path, target: str) -> None:
    """Point `path` at `target`, replacing whatever was there."""
    with contextlib.suppress(FileNotFoundError):
        path.unlink()
    path.symlink_to(target)


def scratch_dir(prefix: str, parent: Path) -> Path:
    """A private directory under a *named* parent.

    `parent` is required, and not defaulted to the system temp: on macOS that
    is `/var/folders/<11>/<24>/T/`, 57 bytes before anything else, which is
    how a socket path came to exceed the platform limit.
    """
    parent.mkdir(parents=True, exist_ok=True)
    return Path(tempfile.mkdtemp(prefix=prefix, dir=parent))


def discard(path: Path) -> None:
    """Remove a tree, tolerating anything.

    Separate from `remove`, which verifies the path is gone and raises if it
    is not. Teardown after a failure runs against whatever state the failure
    left, and turning that into a second exception buries the first.
    """
    shutil.rmtree(path, ignore_errors=True)


def copy_file(source: Path, target: Path) -> None:
    """One file, best-effort.

    Used where the copy is evidence-gathering after a failure: a diagnostic
    that cannot be collected must not replace the failure being diagnosed.
    """
    with contextlib.suppress(OSError):
        shutil.copy(source, target)


def remove(path: Path) -> None:
    """Delete a path, tolerating absence, and verify it is gone.

    A helper as well as an action, because the modules that own machine state
    -- run-history rotation, the workspace, the disk reclaimer -- delete on
    paths where an action cannot reach, and were each repeating the same
    silent `ignore_errors` shape.
    """
    try:
        if path.is_dir() and not path.is_symlink():
            shutil.rmtree(path)
        else:
            path.unlink(missing_ok=True)
    except FileNotFoundError:
        return
    except OSError as failure:
        raise GateError(f"could not remove {path}: {failure}") from failure

    if path.exists() or path.is_symlink():
        raise GateError(
            f"{path} is still present after being removed; the cleanup that "
            "reported success did not happen"
        )
