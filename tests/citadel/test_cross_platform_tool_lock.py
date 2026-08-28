"""Citadel guard for platform-specific Python tool artifacts."""

from __future__ import annotations

import tomllib
from pathlib import Path
from typing import Any

PROJECT_ROOT = Path(__file__).resolve().parents[2]
HADOLINT_VERSION = "2.14.0.1"
HADOLINT_MACOS_SHA256 = "4beaa65cc53bf27fd5faf55267c4a644786184a36efdb7bfbc912af7807dd186"

CROSS_PLATFORM_TOOL_LOCK_RATIONALE = """\
Platform-specific lint tools must be pinned to one reviewed artifact cohort.

Linux cannot exercise a macOS wheel during dependency materialization. The
hadolint-py 2.15.1.2 macOS wheel matched its published SHA-256 and the lockfile
exactly but its ZIP deflate stream was corrupt, so every hosted Mac failed
before tests while Linux stayed green. An open lower bound silently selected
those bytes.

The exact version and reviewed macOS wheel digest below make a cross-platform
tool upgrade an explicit source-contract change. Verify every platform wheel
before changing them; do not widen the dependency back to a range.

See skills/citadel/SKILL.md and build_system/pyproject.toml [dependency-groups].
"""


def _locked_hadolint() -> dict[str, Any]:
    lock = tomllib.loads(
        (PROJECT_ROOT / "build_system/uv.lock").read_text(encoding="utf-8")
    )
    rows = [row for row in lock["package"] if row["name"] == "hadolint-py"]
    assert len(rows) == 1, CROSS_PLATFORM_TOOL_LOCK_RATIONALE
    return rows[0]


def test_hadolint_uses_the_reviewed_exact_cross_platform_cohort() -> None:
    project = tomllib.loads(
        (PROJECT_ROOT / "build_system/pyproject.toml").read_text(encoding="utf-8")
    )
    dependencies = project["dependency-groups"]["dev"]
    requirement = f"hadolint-py=={HADOLINT_VERSION}"

    assert requirement in dependencies, (
        CROSS_PLATFORM_TOOL_LOCK_RATIONALE
        + f"\nexpected exact dependency {requirement!r}; got {dependencies!r}"
    )

    package = _locked_hadolint()
    macos = [wheel for wheel in package["wheels"] if "macosx_10_15_universal2" in wheel["url"]]
    expected_url = (
        "https://files.pythonhosted.org/packages/da/c9/"
        "168c934801c159d61ca0deccb2e24a53f9c389a06446b70149a8f55f69da/"
        "hadolint_py-2.14.0.1-py3-none-macosx_10_15_universal2.whl"
    )
    assert package["version"] == HADOLINT_VERSION, CROSS_PLATFORM_TOOL_LOCK_RATIONALE
    assert len(macos) == 1, CROSS_PLATFORM_TOOL_LOCK_RATIONALE
    assert (
        macos[0]["url"] == expected_url
        and macos[0]["hash"] == f"sha256:{HADOLINT_MACOS_SHA256}"
        and macos[0]["size"] == 20_992_560
    ), CROSS_PLATFORM_TOOL_LOCK_RATIONALE
