"""Read canonical source modes from Git rather than checkout permissions."""

from __future__ import annotations

import subprocess
from pathlib import Path

_REGULAR_FILE_MODES = {
    "100644": 0o644,
    "100755": 0o755,
}


def tracked_source_modes(root: Path, owner: Path) -> dict[str, int]:
    """Return Git's reviewed modes for the regular files below ``owner``."""
    owner_relative = owner.relative_to(root)
    result = subprocess.run(
        (
            "git",
            "ls-files",
            "--stage",
            "-z",
            "--",
            owner_relative.as_posix(),
        ),
        cwd=root,
        check=True,
        capture_output=True,
        text=True,
    )

    modes: dict[str, int] = {}
    for entry in result.stdout.split("\0"):
        if not entry:
            continue
        metadata, path = entry.split("\t", maxsplit=1)
        mode, _object_id, stage = metadata.split()
        if stage != "0" or mode not in _REGULAR_FILE_MODES:
            raise AssertionError(f"unexpected Git index entry for {path}: {metadata}")
        relative = Path(path).relative_to(owner_relative).as_posix()
        modes[relative] = _REGULAR_FILE_MODES[mode]
    return modes
