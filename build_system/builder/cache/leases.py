"""Process-lifetime leases for policy-owned cache generations."""

from __future__ import annotations

import atexit
import fcntl
import os
from collections.abc import Iterable, Iterator
from contextlib import ExitStack, contextmanager
from pathlib import Path
from typing import TYPE_CHECKING, BinaryIO

if TYPE_CHECKING:
    from .paths import CachePaths

_HELD: dict[Path, BinaryIO] = {}

#: Cargo's own cache directory tag, byte for byte. Cargo writes it only into a
#: target directory it creates and `cargo clean` refuses one without it.
CARGO_LOCK_NAME = ".cargo-lock"
CARGO_CACHEDIR_TAG = (
    "Signature: 8a477f597d28d172789f06886806bc55\n"
    "# This file is a cache directory tag created by cargo.\n"
    "# For information about cache directory tags see https://bford.info/cachedir/\n"
)


def retain_path(path: Path) -> BinaryIO:
    """Hold one shared lease until explicit release or process exit."""
    lease = path.absolute()
    existing = _HELD.get(lease)
    if existing is not None and not existing.closed:
        return existing
    lease.parent.mkdir(parents=True, exist_ok=True)
    descriptor = os.fdopen(os.open(lease, os.O_APPEND | os.O_CREAT | os.O_RDWR, 0o600), "a+b")
    try:
        fcntl.flock(descriptor, fcntl.LOCK_SH | fcntl.LOCK_NB)
    except BaseException:
        descriptor.close()
        raise
    _HELD[lease] = descriptor
    return descriptor


def retain_generation(paths: CachePaths, stage_id: str, key: str) -> BinaryIO:
    """Hold the configured lease for one managed stage generation."""
    stage = paths.policy.stages[stage_id]
    if stage.lease_template is None:
        raise ValueError(f"cache stage {stage_id!r} has no generation lease")
    return retain_path(paths.stage(stage_id) / stage.lease_template.format(key=key))


def release_path(path: Path) -> None:
    """Release a retained path, primarily for bounded test fixtures."""
    descriptor = _HELD.pop(path.absolute(), None)
    if descriptor is not None:
        descriptor.close()


def release_all() -> None:
    """Close every process lease before interpreter resource accounting runs."""
    descriptors = tuple(_HELD.values())
    _HELD.clear()
    for descriptor in descriptors:
        descriptor.close()


def active_path(path: Path) -> bool:
    """Return whether another process holds a shared lease at this path."""
    lease = path.absolute()
    if not lease.is_file() or lease.is_symlink():
        return False
    with lease.open("rb") as descriptor:
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            return True
        fcntl.flock(descriptor, fcntl.LOCK_UN)
    return False


@contextmanager
def mutation_locks(paths: CachePaths, stage_ids: Iterable[str]) -> Iterator[tuple[Path, ...]]:
    """Hold producer-owned locks through removal, including a late-starting build."""
    locked = []
    with ExitStack() as stack:
        for stage_id in sorted(set(stage_ids)):
            root = paths.stage(stage_id)
            for relative in sorted(paths.policy.stages[stage_id].mutation_locks):
                lock = root / relative
                if not lock.parent.resolve().is_relative_to(root.resolve()) or lock.is_symlink():
                    raise ValueError(f"mutation lock escapes cache stage: {lock}")
                lock.parent.mkdir(parents=True, exist_ok=True)
                if lock.name == CARGO_LOCK_NAME:
                    _tag_cargo_target(lock.parent.parent)
                descriptor = stack.enter_context(os.fdopen(
                    os.open(lock, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600), "a+b",
                ))
                try:
                    fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
                except BlockingIOError as error:
                    raise ValueError(f"cache stage is busy: {lock}") from error
                locked.append(lock)
        yield tuple(locked)


def _tag_cargo_target(root: Path) -> None:
    """Tag `<root>`, which holding `<root>/<profile>/.cargo-lock` created before
    Cargo could: untagged, cargo-llvm-cov's clean aborted and instrumented
    binaries from earlier checkouts were counted as uncovered code."""
    tag = root / "CACHEDIR.TAG"
    if tag.is_symlink():
        raise ValueError(f"Cargo cache tag is a symlink: {tag}")
    if not tag.exists():
        tag.write_text(CARGO_CACHEDIR_TAG, encoding="utf-8")


atexit.register(release_all)
