"""A value declared by config must be read, never restated in a test.

Seven gate failures in one session had one shape: a test hardcoded a number or
string that a config file authoritatively declares, the config moved, and the
test failed while reporting a broken product.

    coverage floor        asserted 65      justfile said 63
    guest kernel          demanded >= 7    build.toml pinned 6.18
    docker free space     fixture 30 GiB   policy floor rose to 40
    benchmark retention   asserted (1, 6)  Cargo.toml said 0.6
    release fixtures      spelled 1.6.x    RELEASE_LINE said 0.6
    service /version      startswith("1.") Cargo.toml said 0.6.0

None was a defect in Capsem. Each cost between three minutes and a full
forty-minute gate to discover, and each read as a release breaking rather than
a literal outliving the value it tracked.

This test enumerates the authorities and fails when a test restates one of
their current values. Two ways to satisfy it, both better than a literal:

  read it       tomllib.load(...)["workspace"]["package"]["version"]
  or diverge    use a fixture value that is obviously not production
                ("9.9.9"), which also proves the code under test does not
                secretly depend on the real one.
"""

from __future__ import annotations

import re
import tomllib
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parent.parent
SEARCH_ROOTS = (PROJECT_ROOT / "tests", PROJECT_ROOT / "scripts")


def _authorities() -> dict[str, tuple[str, Path]]:
    """Current value -> (what declares it, declaring file)."""
    found: dict[str, tuple[str, Path]] = {}

    cargo = PROJECT_ROOT / "Cargo.toml"
    version = tomllib.loads(cargo.read_text(encoding="utf-8"))["workspace"]["package"][
        "version"
    ]
    found[version] = ("the workspace version", cargo)

    release_binaries = PROJECT_ROOT / "scripts" / "release-binaries.py"
    line = re.search(
        r'^RELEASE_LINE = "([^"]+)"', release_binaries.read_text(encoding="utf-8"), re.M
    )
    if line:
        found[line.group(1)] = ("the release line", release_binaries)

    build = PROJECT_ROOT / "config" / "docker" / "image" / "build.toml"
    branches: set[str] = set()

    def walk(table: dict) -> None:
        for key, value in table.items():
            if isinstance(value, dict):
                walk(value)
            elif key == "kernel_branch":
                branches.add(value)

    walk(tomllib.loads(build.read_text(encoding="utf-8")))
    for branch in branches:
        found[branch] = ("the guest kernel branch", build)

    return found


def _reads(text: str, declaring: Path) -> bool:
    """Whether a file reads its authority rather than restating it."""
    return declaring.name in text


def test_no_test_restates_a_value_its_config_declares() -> None:
    authorities = _authorities()
    assert authorities, "no authoritative values found; this guard would pass vacuously"

    offenders: list[str] = []
    for root in SEARCH_ROOTS:
        for path in sorted(root.rglob("*.py")):
            if path.name == Path(__file__).name:
                continue
            text = path.read_text(encoding="utf-8", errors="ignore")
            for value, (what, declaring) in authorities.items():
                if path == declaring or _reads(text, declaring):
                    continue
                # Word-bounded so 0.6 does not match inside 10.6.2.
                if re.search(rf"(?<![\w.]){re.escape(value)}(?![\w.])", text):
                    offenders.append(
                        f"{path.relative_to(PROJECT_ROOT)} spells {value!r}, which is "
                        f"{what} declared by {declaring.relative_to(PROJECT_ROOT)}"
                    )

    assert not offenders, (
        "these restate a value their config already declares, so the config "
        "moving breaks them and reports a broken product:\n  "
        + "\n  ".join(offenders)
    )
