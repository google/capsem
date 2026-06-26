"""Citadel guards for hot-path dev build optimization.

Manual release validation and `just install` use debug binaries. If protocol
codecs silently fall back to opt-level 0, route latency and gateway benchmarks
look like runtime regressions even though the compiler contract is broken.
"""

from __future__ import annotations

import tomllib
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]

HOT_DEV_OPTIMIZED_PACKAGES = {
    "blake3",
    "serde",
    "serde_core",
    "serde_json",
    "rmp",
    "rmp-serde",
    "hickory-proto",
    "memchr",
    "itoa",
    "ryu",
}

RATIONALE = """\
Hot codec package lost dev optimization.

Capsem's manual release path uses debug binaries from `just install`, so JSON,
MessagePack, DNS wire parsing, and BLAKE3 must stay optimized in the dev
profile. If this guard fails, route/gateway latency can regress from compiler
configuration before the DB or network code even runs.
"""


def test_hot_wire_codecs_are_optimized_in_dev_profile() -> None:
    cargo_toml = tomllib.loads((PROJECT_ROOT / "Cargo.toml").read_text())
    packages = cargo_toml["profile"]["dev"]["package"]
    missing_or_slow = sorted(
        package
        for package in HOT_DEV_OPTIMIZED_PACKAGES
        if packages.get(package, {}).get("opt-level") != 3
    )

    assert not missing_or_slow, RATIONALE + "\nMissing opt-level=3: " + ", ".join(
        missing_or_slow
    )
