#!/usr/bin/env python3
"""Render the GitHub release notes for a binary release.

This was a heredoc in `create-release`. Its tag was unquoted, so the backticks
around the commit hash were command substitution: bash ran the hash as a
program and substituted nothing, leaving "Qualified source: ." and exiting 0.

A missing value is the failure worth catching, and a shell heredoc cannot
catch it -- an unset variable expands to empty and the notes ship incomplete.
Here it raises.
"""

from __future__ import annotations

import os
import sys

CHANGELOG = "https://github.com/{repository}/blob/{commit}/CHANGELOG.md"


def render(*, commit: str, repository: str, manifest_url: str) -> str:
    for name, value in (
        ("SOURCE_COMMIT", commit),
        ("GITHUB_REPOSITORY", repository),
        ("ASSET_MANIFEST_URL", manifest_url),
    ):
        if not value.strip():
            raise SystemExit(f"cannot write release notes: {name} is empty")

    changelog = CHANGELOG.format(repository=repository, commit=commit)
    return (
        f"Qualified source: `{commit}`.\n"
        "\n"
        f"See [CHANGELOG.md]({changelog}) for details.\n"
        "\n"
        "### VM Assets\n"
        "\n"
        f"VM assets are released independently at {manifest_url}.\n"
    )


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        raise SystemExit("usage: write-release-notes.py <output-path>")

    notes = render(
        commit=os.environ.get("SOURCE_COMMIT", ""),
        repository=os.environ.get("GITHUB_REPOSITORY", ""),
        manifest_url=os.environ.get("ASSET_MANIFEST_URL", ""),
    )
    with open(argv[1], "w", encoding="utf-8") as handle:
        handle.write(notes)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
