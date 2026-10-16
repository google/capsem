"""Release preflight must accept the actual checkout before qualification."""

import tomllib
from pathlib import Path

import pytest
from capsem_builder.release.tools import release_binaries

ROOT = Path(__file__).resolve().parents[2]
COHORT_RATIONALE = (
    "The actual publisher must accept this checkout's version cohort before "
    "qualification: it once passed with a stale release.line setting."
)


def workspace_version() -> str:
    return tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]


def test_release_preflight_accepts_the_actual_version_cohort() -> None:
    try:
        release_binaries._validate_version_cohort(workspace_version())
    except RuntimeError as error:
        pytest.fail(f"{COHORT_RATIONALE}: {error}")


def test_a_different_configured_release_line_is_rejected(monkeypatch: pytest.MonkeyPatch) -> None:
    version = workspace_version()
    major, minor, _ = version.split(".", 2)
    monkeypatch.setattr(release_binaries, "RELEASE_LINE", f"{major}.{int(minor) + 1}")
    with pytest.raises(RuntimeError, match="release line"):
        release_binaries._validate_version_cohort(version)
