#!/usr/bin/env python3
"""Hash the complete non-ignored Git source state without requiring it clean."""

from __future__ import annotations

import argparse
import os
import stat
import subprocess
from pathlib import Path

import blake3

ROOT = Path(__file__).resolve().parents[1]


def source_state_digest(root: Path = ROOT) -> str:
    listed = subprocess.run(
        ("git", "ls-files", "-co", "--exclude-standard", "-z"),
        cwd=root,
        check=True,
        stdout=subprocess.PIPE,
    ).stdout
    paths = sorted(path for path in listed.split(b"\0") if path)
    digest = blake3.blake3()
    for encoded in paths:
        relative = os.fsdecode(encoded)
        path = root / relative
        metadata = path.lstat()
        digest.update(encoded)
        digest.update(b"\0")
        digest.update(f"{stat.S_IMODE(metadata.st_mode):04o}".encode())
        digest.update(b"\0")
        if path.is_symlink():
            digest.update(b"link\0")
            digest.update(os.fsencode(os.readlink(path)))
        elif path.is_file():
            digest.update(b"file\0")
            with path.open("rb") as handle:
                while chunk := handle.read(1024 * 1024):
                    digest.update(chunk)
        else:
            raise ValueError(f"unsupported non-file source entry: {relative}")
        digest.update(b"\0")
    return digest.hexdigest()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root",
        type=Path,
        default=ROOT,
        help=(
            "the tree to hash; defaults to the checkout this script lives in. "
            "A run working from a private copy has to be able to hash the "
            "checkout it was copied from, which is a different tree."
        ),
    )
    print(source_state_digest(parser.parse_args().root.resolve()))
