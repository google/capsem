#!/usr/bin/env python3
"""Optionally render pending binary notes without changing release state."""

import argparse
import re
import sys
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


def render_release_notes(changelog: str, version: str) -> str:
    """Bind pending notes to a version; the remote tag performs the release."""
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise ValueError("version must be numeric SemVer")
    body = validate_unreleased(changelog)
    return f"version: {version}\n---\n{body}\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version")
    parser.add_argument(
        "--check",
        action="store_true",
        help="validate that [Unreleased] contains publishable notes without writing",
    )
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
        rendered = render_release_notes(changelog, args.version)
    except (OSError, ValueError) as error:
        print(error, file=sys.stderr)
        return 1
    args.output.write_text(rendered, encoding="utf-8")
    print(f"{args.output} updated for v{args.version}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
