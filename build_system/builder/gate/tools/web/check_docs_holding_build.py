"""Fail unless docs contain only release-line holding pages derived from the manual."""

from __future__ import annotations

import argparse
import tomllib
from pathlib import Path

from . import repository_root


class HoldingBuildError(RuntimeError):
    """The built docs artifact exposed something beyond the holding page."""


TOMBSTONE_MARKERS = (
    "Documentation route unavailable",
    "pre-release qualification",
    "Documentation is being prepared",
    'name="robots" content="noindex, nofollow"',
)


def _required_markers() -> tuple[str, ...]:
    config = repository_root() / "config" / "gate.toml"
    release_line = tomllib.loads(config.read_text(encoding="utf-8"))["release"]["line"]
    return (
        f"Capsem {release_line} documentation",
        "pre-release qualification",
        "Documentation is being prepared",
    )


FORBIDDEN_CONTENT = (
    ("releases/latest", "releases/latest"),
    ("install.sh", "install.sh"),
    ('href="/getting-started', 'href="/getting-started'),
    ('href="/architecture/', 'href="/architecture/'),
    ('href="/security/', 'href="/security/'),
    ('href="/usage/', 'href="/usage/'),
    ('href="/benchmarks/', 'href="/benchmarks/'),
    ('href="/debugging/', 'href="/debugging/'),
    ('href="/development/', 'href="/development/'),
    ('href="/releases/', 'href="/releases/'),
    ("getting started", "Getting Started"),
    ("starlight", "Starlight"),
)


def expected_tombstones(source_root: Path) -> tuple[str, ...]:
    """Return the output file owned by each non-root manual source."""
    if not source_root.is_dir():
        raise HoldingBuildError(f"docs source directory is missing: {source_root}")

    outputs: list[str] = []
    for source in sorted(source_root.rglob("*")):
        if not source.is_file() or source.suffix not in {".md", ".mdx"}:
            continue
        lines = source.read_text(encoding="utf-8").splitlines()
        if lines and lines[0] == "---":
            for line in lines[1:]:
                if line == "---":
                    break
                key, separator, _value = line.partition(":")
                if separator and key.strip() == "slug" and not line[:1].isspace():
                    relative = source.relative_to(source_root).as_posix()
                    raise HoldingBuildError(
                        f"{relative} has a custom slug that the holding route does not materialize"
                    )
        route = source.relative_to(source_root).with_suffix("")
        if route.name == "index":
            route = route.parent
        if route == Path("."):
            continue
        outputs.append((route / "index.html").as_posix())

    duplicates = sorted(path for path in set(outputs) if outputs.count(path) > 1)
    if duplicates:
        raise HoldingBuildError(f"manual sources map to duplicate routes: {', '.join(duplicates)}")
    return tuple(outputs)


def _verify_markers(relative: str, html: str, markers: tuple[str, ...]) -> None:
    missing = [marker for marker in markers if marker not in html]
    if missing:
        raise HoldingBuildError(f"{relative} is missing: {', '.join(missing)}")


def _verify_retired_content(relative: str, html: str) -> None:
    lowered = html.lower()
    leaked = [label for needle, label in FORBIDDEN_CONTENT if needle in lowered]
    if leaked:
        raise HoldingBuildError(f"{relative} exposes retired content: {', '.join(leaked)}")


def verify_holding_build(dist: Path, source_root: Path) -> None:
    """Verify root, 404, headers, and the exact source-derived tombstones."""
    if not dist.is_dir():
        raise HoldingBuildError(f"docs output directory is missing: {dist}")

    built_files = sorted(
        path.relative_to(dist).as_posix() for path in dist.rglob("*") if path.is_file()
    )
    tombstones = expected_tombstones(source_root)
    expected_files = sorted(("404.html", "_headers", "index.html", *tombstones))
    if built_files != expected_files:
        missing = sorted(set(expected_files) - set(built_files))
        unexpected = sorted(set(built_files) - set(expected_files))
        details = []
        if missing:
            details.append(f"missing: {', '.join(missing)}")
        if unexpected:
            details.append(f"unexpected: {', '.join(unexpected)}")
        raise HoldingBuildError("docs output inventory mismatch; " + "; ".join(details))

    index = (dist / "index.html").read_text(encoding="utf-8")
    _verify_markers("index.html", index, _required_markers())

    not_found = (dist / "404.html").read_text(encoding="utf-8")
    _verify_markers(
        "404.html",
        not_found,
        ("Documentation route unavailable", "pre-release qualification"),
    )

    headers = (dist / "_headers").read_text(encoding="utf-8")
    _verify_markers(
        "_headers",
        headers,
        (
            "Cache-Control: no-store",
            "CDN-Cache-Control: no-store",
            "Cloudflare-CDN-Cache-Control: no-store",
            "X-Robots-Tag: noindex, nofollow",
        ),
    )

    html_files = {"index.html": index, "404.html": not_found}
    for relative in tombstones:
        html = (dist / relative).read_text(encoding="utf-8")
        _verify_markers(relative, html, TOMBSTONE_MARKERS)
        html_files[relative] = html

    for relative, html in html_files.items():
        _verify_retired_content(relative, html)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("dist", type=Path, help="built docs output directory")
    parser.add_argument("source_root", type=Path, help="detailed docs source directory")
    args = parser.parse_args()
    try:
        verify_holding_build(args.dist, args.source_root)
    except HoldingBuildError as error:
        parser.error(str(error))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
