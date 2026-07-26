#!/usr/bin/env python3
"""Require two flat immutable publication directories to contain identical bytes."""

from __future__ import annotations

import argparse
import hashlib
import sys
from pathlib import Path


def publication_files(root: Path, label: str) -> dict[str, Path]:
    if root.is_symlink() or not root.is_dir():
        raise ValueError(f"{label} publication root is missing or unsafe: {root}")
    files: dict[str, Path] = {}
    for entry in root.iterdir():
        if entry.is_symlink() or not entry.is_file():
            raise ValueError(f"{label} publication contains unsafe entry: {entry}")
        files[entry.name] = entry
    if not files:
        raise ValueError(f"{label} publication contains no files")
    return files


def file_identity(path: Path) -> tuple[int, str]:
    digest = hashlib.sha256()
    size = 0
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            size += len(block)
            digest.update(block)
    return size, digest.hexdigest()


def verify_identical_publication(expected: Path, actual: Path) -> None:
    expected_files = publication_files(expected, "expected")
    actual_files = publication_files(actual, "existing")
    expected_names = set(expected_files)
    actual_names = set(actual_files)
    if expected_names != actual_names:
        raise ValueError(
            "immutable publication file set mismatch: "
            f"missing={sorted(expected_names - actual_names)} "
            f"extra={sorted(actual_names - expected_names)}"
        )
    mismatches = [
        name
        for name in sorted(expected_names)
        if file_identity(expected_files[name]) != file_identity(actual_files[name])
    ]
    if mismatches:
        raise ValueError(f"immutable publication byte mismatch: {mismatches}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--expected", type=Path, required=True)
    parser.add_argument("--actual", type=Path, required=True)
    args = parser.parse_args()
    try:
        verify_identical_publication(args.expected, args.actual)
    except (OSError, ValueError) as error:
        print(f"immutable publication verification failed: {error}", file=sys.stderr)
        return 1
    print(f"immutable publication matches exactly: {args.actual}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
