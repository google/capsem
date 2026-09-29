"""Citadel guards for hot-path dev build optimization.

Hands-on local validation through `just install` uses debug binaries. If protocol
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

Capsem's hands-on local install path uses debug binaries, so JSON, MessagePack,
DNS wire parsing, and BLAKE3 must stay optimized in the dev profile. If this
guard fails, route/gateway latency can regress from compiler configuration
before the DB or network code even runs.
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


#: Debug levels that keep backtraces' file:line without the variable DWARF.
LEAN_DEBUGINFO = ("line-tables-only", "none", 0, False)

DEBUGINFO_RATIONALE = """\
The dev profile emits full debug information again.

Every checkout and every gate prefix compiles the workspace into the one
shared Cargo target, and workspace units are salted by checkout path, so debug
bytes are paid once per checkout. Measured on the Linux build box on
2026-09-29: DWARF was 77% of the test executables in `debug/deps` (8.5 of 11.0
GB sampled) and 41% of the incremental object files, while the stage sat at its
180 GiB maximum and the disk filled four times in two days. `line-tables-only`
keeps file:line in every backtrace and panic, which is what tests and CI read.
When a debugger needs variables, build locally with CARGO_PROFILE_DEV_DEBUG=true.
"""


def _lean(level: object) -> bool:
    # `True == 1` would otherwise pass as `False == 0` does; compare by type too.
    return any(type(level) is type(lean) and level == lean for lean in LEAN_DEBUGINFO)


def full_debuginfo(cargo_toml: dict) -> list[str]:
    """Every dev/test profile or dev package override that emits more than line tables."""
    profiles = cargo_toml.get("profile", {})
    dev = profiles.get("dev", {})
    # Cargo's dev default is 2 (full), and `test` inherits whatever `dev` says.
    found = [] if _lean(dev.get("debug", 2)) else ["profile.dev"]
    test = profiles.get("test", {})
    if "debug" in test and not _lean(test["debug"]):
        found.append("profile.test")
    for name, package in sorted(dev.get("package", {}).items()):
        if "debug" in package and not _lean(package["debug"]):
            found.append(f"profile.dev.package.{name}")
    return found


def test_dev_profile_keeps_only_line_tables() -> None:
    cargo_toml = tomllib.loads((PROJECT_ROOT / "Cargo.toml").read_text())
    offenders = full_debuginfo(cargo_toml)
    assert not offenders, DEBUGINFO_RATIONALE + "\nFull debuginfo: " + ", ".join(offenders)


def test_debuginfo_guard_rejects_every_full_spelling() -> None:
    lean = {"profile": {"dev": {"debug": "line-tables-only"}}}
    assert full_debuginfo(lean) == []
    assert full_debuginfo({}) == ["profile.dev"], "an absent key is Cargo's full default"
    for spelling in (2, 1, True, "full", "limited"):
        assert full_debuginfo({"profile": {"dev": {"debug": spelling}}}) == ["profile.dev"]
    via_test = {"profile": {"dev": {"debug": "line-tables-only"}, "test": {"debug": True}}}
    assert full_debuginfo(via_test) == ["profile.test"]
    via_package = {
        "profile": {"dev": {"debug": 0, "package": {"capsem-core": {"debug": "full"}}}}
    }
    assert full_debuginfo(via_package) == ["profile.dev.package.capsem-core"]
