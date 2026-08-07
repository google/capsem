#!/usr/bin/env python3
"""Bound the checked-in benchmark history without losing its two uses.

The history answers two different questions, and they need different amounts of
data:

  * "did this release regress against the last one?" -- one recording per
    release is enough, and keeping more just grows the repository.
  * "what threshold would not flap?" -- that needs several recordings of the
    *current* release, because a floor set from a single sample is a guess.

So: keep every recording of the current version, and only the newest recording
of each older one. Recordings are grouped by what makes them comparable --
category, series, architecture -- because an arm64 number and an x86_64 number
are not two samples of the same thing.

Named baselines (baseline.json, post_t3_debug_reference.json) are deliberate
reference points rather than routine output, and are never pruned.

Dry run by default; pass --apply to delete.
"""

from __future__ import annotations

import argparse
import re
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BENCHMARKS = ROOT / "benchmarks"

# <series>_<major>.<minor>.<timestamp>[_<arch>].json -- the timestamp is what
# makes a file routine output rather than a curated baseline.
RECORDING = re.compile(
    r"^(?P<series>.+?)_(?P<major>\d+)\.(?P<minor>\d+)\.(?P<ts>\d{6,})"
    r"(?:_(?P<arch>[\w-]+))?\.json$"
)


def current_version(root: Path = ROOT) -> tuple[int, int]:
    text = (root / "Cargo.toml").read_text(encoding="utf-8")
    match = re.search(r'(?m)^version\s*=\s*"(\d+)\.(\d+)', text)
    if match is None:
        raise SystemExit("could not read the workspace version from Cargo.toml")
    return int(match.group(1)), int(match.group(2))


def plan(benchmarks: Path, keep_version: tuple[int, int]) -> list[Path]:
    """Files to delete: superseded recordings of non-current versions."""
    groups: dict[tuple, list[tuple[int, Path]]] = defaultdict(list)
    for path in sorted(benchmarks.rglob("*.json")):
        match = RECORDING.match(path.name)
        if match is None:
            continue  # curated baseline, or a shape we do not recognise
        version = (int(match.group("major")), int(match.group("minor")))
        if version == keep_version:
            continue  # current release: every sample earns its place
        key = (
            path.parent,
            match.group("series"),
            match.group("arch") or "",
            version,
        )
        groups[key].append((int(match.group("ts")), path))

    superseded: list[Path] = []
    for entries in groups.values():
        entries.sort()
        superseded.extend(path for _, path in entries[:-1])
    return sorted(superseded)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--apply",
        action="store_true",
        help="delete the superseded recordings (default is a dry run)",
    )
    args = parser.parse_args()

    keep = current_version()
    superseded = plan(BENCHMARKS, keep)
    total = sum(1 for _ in BENCHMARKS.rglob("*.json"))

    if not superseded:
        print(f"benchmark history already minimal: {total} files, nothing superseded")
        return 0

    freed = sum(path.stat().st_size for path in superseded)
    for path in superseded:
        print(f"{'delete' if args.apply else 'would delete'} {path.relative_to(ROOT)}")
        if args.apply:
            path.unlink()

    kept = total - (len(superseded) if args.apply else 0)
    print(
        f"\n{len(superseded)} superseded recordings, {freed / 1024:.0f} KiB; "
        f"keeping every {keep[0]}.{keep[1]} sample and the newest of each older release"
        f" -> {kept} files"
    )
    if not args.apply:
        print("dry run; pass --apply to delete")
    return 0


if __name__ == "__main__":
    sys.exit(main())
