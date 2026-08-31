#!/usr/bin/env python3
"""Keep every Rust crate visible behind the workspace coverage average.

The workspace floor catches catastrophic suite loss, but an aggregate can stay
green while one small crate loses every useful assertion.  This checker reads
the LCOV report produced by the same cargo-llvm-cov run and applies the
config-owned per-crate floors.  It also rejects excessive headroom so improved
coverage must ratchet the floor upward instead of silently becoming disposable.
"""

from __future__ import annotations

import argparse
import sys
import tomllib
from collections import defaultdict
from pathlib import Path
from typing import NamedTuple


class Coverage(NamedTuple):
    hit: int
    found: int

    @property
    def percent(self) -> float:
        return 100.0 * self.hit / self.found if self.found else 0.0


def workspace_crates(root: Path, crate_root: Path) -> dict[str, str]:
    """Map a workspace crate directory to its package name."""
    crates: dict[str, str] = {}
    for manifest in sorted((root / crate_root).glob("*/Cargo.toml")):
        package = tomllib.loads(manifest.read_text())["package"]["name"]
        crates[manifest.parent.name] = package
    return crates


def lcov_by_crate(
    report: Path,
    root: Path,
    crate_root: Path,
    crates: dict[str, str],
) -> dict[str, Coverage]:
    """Read unique source-line hits, grouped by owning workspace crate."""
    lines: dict[str, dict[tuple[str, int], int]] = defaultdict(dict)
    owner: str | None = None
    source = ""
    crate_root_names = set(crates)
    crate_root_parts = crate_root.parts

    for raw in report.read_text().splitlines():
        if raw.startswith("SF:"):
            source_path = Path(raw[3:])
            if source_path.is_absolute():
                try:
                    source_path = source_path.relative_to(root.resolve())
                except ValueError:
                    owner = None
                    continue
            parts = source_path.parts
            prefix_len = len(crate_root_parts)
            owner = (
                parts[prefix_len]
                if len(parts) >= prefix_len + 2 and parts[:prefix_len] == crate_root_parts
                else None
            )
            if owner not in crate_root_names:
                owner = None
                continue
            source = source_path.as_posix()
        elif owner is not None and raw.startswith("DA:"):
            position, count, *_ = raw[3:].split(",")
            key = (source, int(position))
            lines[owner][key] = max(lines[owner].get(key, 0), int(count))

    return {
        crates[directory]: Coverage(
            hit=sum(count > 0 for count in measured.values()),
            found=len(measured),
        )
        for directory, measured in lines.items()
    }


def violations(
    measured: dict[str, Coverage],
    expected_crates: set[str],
    floors: dict[str, float],
    max_headroom: float,
) -> list[str]:
    problems: list[str] = []
    configured = set(floors)
    if configured != expected_crates:
        missing = sorted(expected_crates - configured)
        stale = sorted(configured - expected_crates)
        if missing:
            problems.append(f"coverage floors missing workspace crates: {missing}")
        if stale:
            problems.append(f"coverage floors name stale workspace crates: {stale}")

    reported = set(measured)
    missing_report = sorted(expected_crates - reported)
    if missing_report:
        problems.append(f"LCOV report omitted workspace crates: {missing_report}")

    for crate in sorted(expected_crates & configured & reported):
        floor = floors[crate]
        coverage = measured[crate]
        if not 0.0 <= floor <= 100.0:
            problems.append(f"{crate}: invalid coverage floor {floor:.2f}%")
        elif coverage.percent + 1e-9 < floor:
            problems.append(
                f"{crate}: {coverage.percent:.2f}% is below its {floor:.2f}% floor "
                f"({coverage.hit}/{coverage.found} lines)"
            )
        elif coverage.percent - floor > max_headroom + 1e-9:
            minimum = coverage.percent - max_headroom
            problems.append(
                f"{crate}: {coverage.percent:.2f}% leaves {coverage.percent - floor:.2f} "
                f"points above its {floor:.2f}% floor; raise the floor to at least "
                f"{minimum:.2f}% in the same change"
            )
    return problems


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--crate-root", required=True, type=Path)
    parser.add_argument("--config", default=Path("config/gate.toml"), type=Path)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    root = Path.cwd()
    settings = tomllib.loads((root / args.config).read_text())["modules"]
    floors = {
        crate: float(floor)
        for crate, floor in settings["rust_coverage_crate_floors"].items()
    }
    max_headroom = float(settings["rust_coverage_ratchet_headroom"])
    crates = workspace_crates(root, args.crate_root)
    measured = lcov_by_crate(root / args.report, root, args.crate_root, crates)
    problems = violations(measured, set(crates.values()), floors, max_headroom)
    if problems:
        print(
            "Per-crate Rust coverage ratchet failed. The workspace average cannot "
            "hide a crate regression:\n  " + "\n  ".join(problems),
            file=sys.stderr,
        )
        return 1

    for crate in sorted(measured):
        coverage = measured[crate]
        print(
            f"{crate:24} {coverage.percent:6.2f}% "
            f"({coverage.hit}/{coverage.found}, floor {floors[crate]:.2f}%)"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
