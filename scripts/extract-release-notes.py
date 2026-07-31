#!/usr/bin/env python3
"""Promote Unreleased notes and render LATEST_RELEASE.md for a binary release."""

import argparse
import re
import sys
from datetime import date
from pathlib import Path


def validate_unreleased(changelog: str) -> str:
    heading = re.search(r"^## \[Unreleased\]\s*$", changelog, re.MULTILINE)
    if heading is None:
        raise ValueError("CHANGELOG.md has no [Unreleased] section")
    following = re.search(r"^## \[", changelog[heading.end() :], re.MULTILINE)
    if following is None:
        raise ValueError("CHANGELOG.md has no versioned section after [Unreleased]")
    body_end = heading.end() + following.start()
    body = changelog[heading.end() : body_end].strip()
    if not body:
        raise ValueError("CHANGELOG.md [Unreleased] section is empty")
    return body


def promote_release(changelog: str, version: str, release_date: str) -> tuple[str, str]:
    heading = re.search(r"^## \[Unreleased\]\s*$", changelog, re.MULTILINE)
    if re.search(rf"^## \[{re.escape(version)}\]", changelog, re.MULTILINE):
        raise ValueError(f"CHANGELOG.md already contains release {version}")
    body = validate_unreleased(changelog)
    assert heading is not None
    following = re.search(r"^## \[", changelog[heading.end() :], re.MULTILINE)
    assert following is not None
    body_end = heading.end() + following.start()
    replacement = (
        f"## [Unreleased]\n\n"
        f"## [{version}] - {release_date}\n\n"
        f"{body}\n\n"
    )
    updated = changelog[: heading.start()] + replacement + changelog[body_end:].lstrip()
    return updated, body


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version")
    parser.add_argument(
        "--check",
        action="store_true",
        help="validate that [Unreleased] contains publishable notes without writing",
    )
    parser.add_argument("--date", default=date.today().isoformat())
    parser.add_argument("--changelog", type=Path, default=Path("CHANGELOG.md"))
    parser.add_argument("--output", type=Path, default=Path("LATEST_RELEASE.md"))
    args = parser.parse_args()
    if not args.check and args.version is None:
        parser.error("--version is required unless --check is used")
    if args.version is not None and not re.fullmatch(r"\d+\.\d+\.\d+", args.version):
        parser.error("--version must be numeric SemVer")
    try:
        changelog = args.changelog.read_text(encoding="utf-8")
        if args.check:
            validate_unreleased(changelog)
            print("CHANGELOG.md [Unreleased] release notes are ready")
            return 0
        assert args.version is not None
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
