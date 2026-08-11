#!/usr/bin/env python3
"""Fail unless the docs artifact contains only the 0.6 holding page."""

from __future__ import annotations

import argparse
from pathlib import Path


class HoldingBuildError(RuntimeError):
    """The built docs artifact exposed something beyond the holding page."""


REQUIRED_MARKERS = (
    "Capsem 0.6 documentation",
    "pre-release qualification",
    "Documentation is being prepared",
)

FORBIDDEN_REFERENCES = (
    "releases/latest",
    "install.sh",
    'href="/getting-started',
    'href="/architecture/',
    'href="/security/',
    'href="/usage/',
    'href="/benchmarks/',
    'href="/debugging/',
    'href="/development/',
    'href="/releases/',
)


def verify_holding_build(dist: Path) -> None:
    """Verify the root holding page, platform 404, and no stale docs route."""
    if not dist.is_dir():
        raise HoldingBuildError(f"docs output directory is missing: {dist}")

    built_files = sorted(
        path.relative_to(dist).as_posix() for path in dist.rglob("*") if path.is_file()
    )
    if built_files != ["404.html", "index.html"]:
        rendered = ", ".join(built_files) if built_files else "none"
        raise HoldingBuildError(
            f"docs output must contain only index.html and 404.html; found: {rendered}"
        )

    index = (dist / "index.html").read_text(encoding="utf-8")
    missing = [marker for marker in REQUIRED_MARKERS if marker not in index]
    if missing:
        raise HoldingBuildError(f"docs holding page is missing: {', '.join(missing)}")

    not_found = (dist / "404.html").read_text(encoding="utf-8")
    for marker in ("Documentation route unavailable", "pre-release qualification"):
        if marker not in not_found:
            raise HoldingBuildError(f"docs 404 page is missing: {marker}")

    leaked = [
        reference
        for reference in FORBIDDEN_REFERENCES
        if reference in index or reference in not_found
    ]
    if leaked:
        raise HoldingBuildError(f"docs holding page exposes retired links: {', '.join(leaked)}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("dist", type=Path, help="built docs output directory")
    args = parser.parse_args()
    try:
        verify_holding_build(args.dist)
    except HoldingBuildError as error:
        parser.error(str(error))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
