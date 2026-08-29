"""What `create-release` writes into the GitHub release.

The job has been skipped in every one of the twenty-two binary release
attempts so far, so nothing it does had ever run. These are the first tests
of it.
"""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

import pytest
from capsem_builder.release.tools import write_release_notes as notes

PROJECT_ROOT = Path(__file__).resolve().parents[3]

COMMIT = "4391e6d249f2c0a1b3d4e5f60718293a4b5c6d7e"
FIELDS = {
    "commit": COMMIT,
    "repository": "google/capsem",
    "manifest_url": "https://release.capsem.org/stable/assets.json",
}


def test_the_commit_survives_into_the_notes() -> None:
    """The bug: a heredoc ran it as a command and wrote nothing in its place."""
    body = notes.render(**FIELDS)
    assert f"Qualified source: `{COMMIT}`." in body


def test_the_changelog_link_points_at_the_qualified_commit() -> None:
    body = notes.render(**FIELDS)
    assert f"https://github.com/google/capsem/blob/{COMMIT}/CHANGELOG.md" in body


def test_the_asset_manifest_url_is_named() -> None:
    assert FIELDS["manifest_url"] in notes.render(**FIELDS)


@pytest.mark.parametrize("missing", sorted(FIELDS))
def test_an_empty_value_refuses_instead_of_shipping_a_gap(missing: str) -> None:
    """Shell expanded an unset variable to nothing and exited 0."""
    with pytest.raises(SystemExit, match="cannot write release notes"):
        notes.render(**{**FIELDS, missing: ""})


def test_the_workflow_calls_this_script() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "release.yaml").read_text(
        encoding="utf-8"
    )
    assert "build_system/scripts/release/write-release-notes.py" in workflow
    assert "Qualified source:" not in workflow, "the heredoc is still there"


def test_launcher_forwards_the_output_argument(tmp_path: Path) -> None:
    output = tmp_path / "release-notes.md"
    result = subprocess.run(
        [sys.executable, "-I", "build_system/scripts/release/write-release-notes.py", str(output)],
        cwd=PROJECT_ROOT,
        env={
            **os.environ,
            "SOURCE_COMMIT": COMMIT,
            "GITHUB_REPOSITORY": FIELDS["repository"],
            "ASSET_MANIFEST_URL": FIELDS["manifest_url"],
        },
        text=True,
        capture_output=True,
        check=False,
    )

    assert result.returncode == 0, result.stderr
    assert output.read_text(encoding="utf-8") == notes.render(**FIELDS)
