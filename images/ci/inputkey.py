"""The input key of an official image: everything its build reads, as one digest.

The publication workflow tags each build with its key and skips a build whose
key is already published. So the key must change whenever an input does --
a file's bytes, its name, its executable bit, or (for an agent image) the
base digest it builds FROM -- and must not change for anything else, such as
the order a checkout happened to write files in.

Usage: inputkey.py <path>... [--with <value>]...
"""

from __future__ import annotations

import argparse
import hashlib
import stat
from collections.abc import Sequence
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
#: Bumped when the key's construction changes, so no old tag is mistaken for
#: a build of the same inputs under the new rule.
VERSION = b"capsem-official-image-input-key-v1"
#: Files beside an image that its build never reads: the qualification
#: manifest says how the built image is tested, so editing it re-qualifies the
#: image (tests/qualification) instead of rebuilding it.
NOT_BUILD_INPUTS = frozenset({"qualify.toml"})


def _files(path: Path) -> list[Path]:
    if path.is_file():
        return [path]
    files = sorted(
        entry for entry in path.rglob("*") if entry.is_file() and entry.name not in NOT_BUILD_INPUTS
    )
    if not files:
        raise ValueError(f"{path} holds no files, so it cannot key a build")
    return files


def input_key(paths: Sequence[Path], *, root: Path = ROOT, extra: Sequence[str] = ()) -> str:
    """Hex SHA-256 over every file under `paths`, then each `extra` value."""
    digest = hashlib.sha256(VERSION + b"\0")
    files = sorted({file for path in paths for file in _files(path)})
    for file in files:
        data = file.read_bytes()
        executable = bool(file.stat().st_mode & stat.S_IXUSR)
        for field in (
            b"file",
            file.relative_to(root).as_posix().encode(),
            b"x" if executable else b"-",
            str(len(data)).encode(),
        ):
            digest.update(field + b"\0")
        digest.update(data)
    for value in extra:
        digest.update(b"with\0" + value.encode() + b"\0")
    return digest.hexdigest()


def main(argv: Sequence[str] | None = None) -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("paths", nargs="+", type=Path)
    parser.add_argument("--with", dest="extra", action="append", default=[])
    args = parser.parse_args(argv)
    paths = [path if path.is_absolute() else ROOT / path for path in args.paths]
    print(input_key(paths, extra=args.extra))


if __name__ == "__main__":
    main()
