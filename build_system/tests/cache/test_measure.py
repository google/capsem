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
