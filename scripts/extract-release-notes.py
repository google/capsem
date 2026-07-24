#!/usr/bin/env python3
"""Promote Unreleased notes and render LATEST_RELEASE.md for a binary release."""

import argparse
from datetime import date
import re
import sys
from pathlib import Path


def promote_release(changelog: str, version: str, release_date: str) -> tuple[str, str]:
    heading = re.search(r"^## \[Unreleased\]\s*$", changelog, re.MULTILINE)
    if heading is None:
        raise ValueError("CHANGELOG.md has no [Unreleased] section")
    if re.search(rf"^## \[{re.escape(version)}\]", changelog, re.MULTILINE):
        raise ValueError(f"CHANGELOG.md already contains release {version}")
    following = re.search(r"^## \[", changelog[heading.end() :], re.MULTILINE)
    if following is None:
        raise ValueError("CHANGELOG.md has no versioned section after [Unreleased]")
    body_end = heading.end() + following.start()
    body = changelog[heading.end() : body_end].strip()
    if not body:
        raise ValueError("CHANGELOG.md [Unreleased] section is empty")
    replacement = (
        f"## [Unreleased]\n\n"
        f"## [{version}] - {release_date}\n\n"
        f"{body}\n\n"
    )
    updated = changelog[: heading.start()] + replacement + changelog[body_end:].lstrip()
    return updated, body


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True)
    parser.add_argument("--date", default=date.today().isoformat())
    parser.add_argument("--changelog", type=Path, default=Path("CHANGELOG.md"))
    parser.add_argument("--output", type=Path, default=Path("LATEST_RELEASE.md"))
    args = parser.parse_args()
    if not re.fullmatch(r"\d+\.\d+\.\d+", args.version):
        parser.error("--version must be numeric SemVer")
    try:
        changelog = args.changelog.read_text(encoding="utf-8")
        updated, body = promote_release(changelog, args.version, args.date)
    except (OSError, ValueError) as error:
        print(error, file=sys.stderr)
        return 1
    args.changelog.write_text(updated, encoding="utf-8")
    args.output.write_text(
        f"version: {args.version}\n---\n{body}\n", encoding="utf-8"
    )
    print(f"{args.output} updated for v{args.version}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
