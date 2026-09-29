"""Measuring a cache entry only reads it, and must survive what it cannot read."""

import os
from pathlib import Path

from capsem_builder.cache.measure import measure


def test_an_unreadable_directory_is_dated_not_fatal(tmp_path: Path) -> None:
    """A dead test can leave a mode-000 directory in a cache stage. Measuring
    the stage used to raise PermissionError, which failed every prune of the
    shared test root -- and the release lane with it -- before the removal
    that can take the directory back ever ran."""
    entry = tmp_path / "entry"
    entry.mkdir()
    (entry / "visible").write_bytes(b"abcd")
    sealed = entry / "sealed"
    sealed.mkdir()
    (sealed / "hidden").write_bytes(b"x" * 100)
    os.utime(sealed, ns=(10**18, 10**18))
    sealed.chmod(0o000)
    try:
        measured = measure(entry, set())
    finally:
        sealed.chmod(0o700)

    assert measured.logical_bytes == 4, "only what could be read is counted"
    assert measured.last_used_ns >= 10**18, "the unreadable directory still dates the entry"


def test_a_file_removed_mid_walk_counts_as_gone(tmp_path: Path, monkeypatch) -> None:
    """rustc writes and deletes `*.rcgu.o` temporaries in `deps/` while it
    runs. A release `just test` died before any work on 2026-09-29 because the
    cargo enforcement walk listed one and it was gone by the time it was
    measured (FileNotFoundError). A vanished file holds no bytes."""
    entry = tmp_path / "deps"
    entry.mkdir()
    (entry / "kept.rlib").write_bytes(b"abcd")
    doomed = entry / "unit.rcgu.o"
    doomed.write_bytes(b"x" * 100)
    real_scandir = os.scandir

    class RacingListing:
        """Lists the directory, then lets the compiler delete its temporary."""

        def __init__(self, path) -> None:
            with real_scandir(path) as listing:
                self.entries = list(listing)
            doomed.unlink()

        def __enter__(self):
            return iter(self.entries)

        def __exit__(self, *_exc) -> None:
            return None

    racing_scandir = RacingListing

    monkeypatch.setattr(os, "scandir", racing_scandir)
    measured = measure(entry, set())

    assert measured.logical_bytes == 4


def test_a_whole_entry_removed_before_the_walk_is_empty(tmp_path: Path) -> None:
    gone = tmp_path / "incremental" / "capsem_core-0f3igal4whwc0"

    assert measure(gone, set()).logical_bytes == 0
