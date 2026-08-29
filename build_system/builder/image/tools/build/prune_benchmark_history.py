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
import os
import re
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(os.environ.get("CAPSEM_REPOSITORY_ROOT", Path.cwd())).resolve()
BENCHMARKS = ROOT / "benchmarks" / "baselines"

# <series>_<major>.<minor>.<ordinal>[_<arch>].json -- a version-shaped name is
# what makes a file routine output rather than a curated baseline.
#
# The third component used to be required to be a six-digit timestamp, from
# the retired `1.5.1783712334` scheme. Semver never has six digits there, so
# from `0.6.0` onward every recording fell through to "a shape we do not
# recognise" and became immortal: the policy below described a tree it had
# stopped applying to. It is now any run of digits, which reads both schemes
# and orders both correctly -- the two never share a (major, minor) group, and
# within a group an integer compares right where text does not (`0.5.10` is
# newer than `0.5.9`).
RECORDING = re.compile(
    r"^(?P<series>.+?)_(?P<major>\d+)\.(?P<minor>\d+)\.(?P<ts>\d+)"
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


def summary(
    total: int, superseded: int, keep: tuple[int, int], freed: int | None = None
) -> str:
    """One sentence describing the outcome, the same either way.

    It used to subtract the deletions only when `--apply` was passed, so a dry
    run over 82 files planning to delete 47 of them ended "-> 82 files" -- the
    count before, presented as the count after. The number a person reads to
    decide whether to apply said nothing would change.
    """
    size = "" if freed is None else f", {freed / 1024:.0f} KiB"
    return (
        f"{superseded} superseded recordings{size}; keeping every "
        f"{keep[0]}.{keep[1]} sample and the newest of each older release"
        f" -> {total - superseded} files"
    )


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

    print("\n" + summary(total, len(superseded), keep, freed))
    if not args.apply:
        print("dry run; pass --apply to delete")
    return 0


if __name__ == "__main__":
    sys.exit(main())
