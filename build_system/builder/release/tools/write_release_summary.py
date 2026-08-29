"""Render the release step summary, and refuse a release missing an artifact.

Extracted from a twenty-five line shell body in `create-release`: a `find`, two
`du` calls, an embedded Python one-liner for the SBOM count, a loop building
markdown rows, and `[ -n "$LINUX_ROWS" ]`. That last one is an assertion, not
formatting -- a release reaching this step without a `.deb` has lost an artifact
between jobs, and publishing the rest would ship a channel whose Linux rows
point at nothing. It ran after attestation and before the GitHub release was
created, where a failure is most expensive, and no test could call it.

Two silent fallbacks are now refusals. The shell wrote `N/A` for a missing
package and `?` for an unreadable SBOM, which read as rendering quirks in a
summary nobody checks; both mean an artifact the attestation step covered did
not arrive.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path

#: Powers of 1024, the way `du -h` reports them.
_UNITS = ("B", "K", "M", "G")


def human_size(path: Path) -> str:
    size = float(path.stat().st_size)
    for unit in _UNITS:
        if size < 1024 or unit == _UNITS[-1]:
            return f"{size:.0f}{unit}"
        size /= 1024
    return f"{size:.0f}G"


def render(artifacts: Path, *, tag: str, manifest_url: str) -> str:
    """The summary markdown, or an exit naming what is missing."""
    version = tag.removeprefix("v")

    debs = sorted(artifacts.glob("*.deb"))
    if not debs:
        raise SystemExit(f"no Linux package in {artifacts}; a release must ship every artifact")

    sbom_path = artifacts / "capsem-sbom.spdx.json"
    try:
        packages = len(json.loads(sbom_path.read_text(encoding="utf-8")).get("packages", []))
    except (OSError, ValueError) as error:
        raise SystemExit(f"cannot read the attested SBOM at {sbom_path}: {error}") from error

    rows = []
    for pkg in sorted(artifacts.glob("*.pkg")):
        rows.append(f"| {pkg.name} | {human_size(pkg)} |")
    for deb in debs:
        rows.append(f"| {deb.name} | {human_size(deb)} |")

    table = "\n".join(rows)
    return (
        f"## Release {version}\n\n"
        "### Artifacts\n\n"
        "| File | Size |\n"
        "|------|------|\n"
        f"{table}\n"
        f"| Asset manifest | {manifest_url} |\n"
        f"| capsem-sbom.spdx.json | {packages} packages |\n\n"
        "### Security\n\n"
        "- Apple codesigned (Developer ID), notarized + stapled (.pkg)\n"
        "- SLSA build provenance attested (pkg + deb)\n"
        "- SBOM attested (SPDX 2.3, pkg + deb)\n"
        "- VM assets are served by the manual asset workflow through release.capsem.org\n"
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifacts", type=Path, default=Path("release-artifacts"))
    parser.add_argument("--tag", required=True)
    parser.add_argument("--manifest-url", required=True)
    args = parser.parse_args(argv)

    summary = render(args.artifacts, tag=args.tag, manifest_url=args.manifest_url)
    destination = os.environ.get("GITHUB_STEP_SUMMARY")
    if destination:
        with open(destination, "a", encoding="utf-8") as handle:
            handle.write(summary)
    else:
        sys.stdout.write(summary)
    return 0


if __name__ == "__main__":
    sys.exit(main())
