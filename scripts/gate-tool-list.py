#!/usr/bin/env python3
"""Print a prebuilt-installer tool list for a named set, from config.

Workflows used to spell `cargo-nextest@0.9.137,b3sum@1.8.5` by hand, seven
times, in seven different subsets. The versions happened to agree with
`config/gate.toml`; the membership was guesswork. `b3sum` was in the fast
gate's list and not the binary pairing gate's, though both run the broad suite
and its asset-integrity tests shell out to `b3sum` -- so three of them failed on
a missing tool ten minutes into a release job that had otherwise gone green.

Nobody chose that subset. It was simply never compared with any other, because
nothing related a job's tool list to what the job runs.

So a workflow names a set and this derives the rest. The installable name is
taken from the `install` command rather than the crate name, because they
differ: `cargo-tauri` is installed as `tauri-cli`, which is exactly the sort of
detail a hand-copied list gets wrong.
"""

from __future__ import annotations

import argparse
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def installable(install: list[str]) -> tuple[str, str]:
    """The name and version a prebuilt installer needs, from `cargo install`."""
    if len(install) < 5 or install[:2] != ["cargo", "install"] or install[3] != "--version":
        raise SystemExit(f"unrecognised install command: {install}")
    return install[2], install[4]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sets", required=True, help="comma-separated set names")
    args = parser.parse_args(argv)

    config = tomllib.loads((ROOT / "config" / "gate.toml").read_text(encoding="utf-8"))
    toolchain = config["toolchain"]
    crates = {crate["name"]: crate for crate in toolchain["crates"]}
    declared = toolchain["sets"]

    wanted: list[str] = []
    for label in args.sets.split(","):
        label = label.strip()
        if label not in declared:
            raise SystemExit(f"unknown tool set {label!r}; config declares {sorted(declared)}")
        for name in declared[label]:
            if name not in wanted:
                wanted.append(name)

    print(",".join(f"{n}@{v}" for n, v in (installable(crates[w]["install"]) for w in wanted)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
