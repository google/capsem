"""Prove every row of a published channel manifest downloads and matches.

Extracted from a twenty-line shell body inside `release.yaml`, which carried a
`curl` loop, a byte-size comparison and a blake3 check written as a Python
heredoc indented inside YAML. That is a program no test could call and no
reviewer reads in place -- `[boundary]` already says so of justfile recipes, and
the shape ratchet had it recorded as debt.

It is also the last thing a release does before anyone can install the result,
and it was the only publication step with no coverage at all. The live stable
channel served `status: current` for a month with three dead package URLs,
which is the failure this step exists to catch and nothing proved it could.

Three questions per row, in the order that fails cheapest: is it reachable, is
it the size the manifest claims, and is it the bytes the manifest claims. A
size mismatch is reported rather than short-circuited, because "wrong length"
and "wrong content" are different accidents and a release wants both named.
"""

from __future__ import annotations

import argparse
import json
import sys
import urllib.error
import urllib.request
from pathlib import Path

import blake3

from .list_release_manifest_assets import manifest_asset_rows

#: `release.capsem.org` answers the default `python-urllib` agent with 403,
#: which reads as a dead row for every site-relative path at once.
AGENT = {"User-Agent": "capsem-release-verify/1.0"}

TIMEOUT = 60


def rows(manifest_path: Path, manifest_url: str) -> list[tuple[str, str, int, str]]:
    """(url, expected blake3, expected bytes, label) for every declared asset."""
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    return [
        (url, digest.removeprefix("blake3:"), size, f"{version}/{arch}/{name}")
        for version, arch, name, digest, size, url in manifest_asset_rows(
            manifest, manifest_url
        )
    ]


def fetch(url: str) -> bytes:
    request = urllib.request.Request(url, headers=dict(AGENT))
    with urllib.request.urlopen(request, timeout=TIMEOUT) as response:
        return response.read()


def verify(manifest_path: Path, manifest_url: str) -> list[str]:
    """Every way a row can fail, named per row rather than counted."""
    failures: list[str] = []
    declared = rows(manifest_path, manifest_url)
    if not declared:
        return ["the manifest declares no assets at all"]

    for url, expected_digest, expected_bytes, label in declared:
        try:
            payload = fetch(url)
        except urllib.error.HTTPError as error:
            failures.append(f"{label} is not reachable (HTTP {error.code}): {url}")
            continue
        except Exception as error:  # the failure is the finding
            failures.append(f"{label} could not be fetched ({error!r}): {url}")
            continue
        if len(payload) != expected_bytes:
            failures.append(
                f"{label} is {len(payload)} bytes, the manifest declares "
                f"{expected_bytes}: {url}"
            )
        actual = blake3.blake3(payload).hexdigest()
        if actual != expected_digest:
            failures.append(
                f"{label} hashes to {actual}, the manifest declares {expected_digest}: {url}"
            )
    return failures


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest-url", required=True)
    parser.add_argument(
        "--manifest-path",
        type=Path,
        help="a manifest already on disk; otherwise it is fetched from --manifest-url",
    )
    args = parser.parse_args(argv)

    path = args.manifest_path
    if path is None:
        path = Path("/tmp/verify-channel-manifest.json")
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(fetch(args.manifest_url))

    failures = verify(path, args.manifest_url)
    if failures:
        for failure in failures:
            print(f"::error::{failure}")
        print(f"{len(failures)} manifest row(s) did not verify", file=sys.stderr)
        return 1
    print(f"every row of {args.manifest_url} downloads and matches its digest")
    return 0


if __name__ == "__main__":
    sys.exit(main())
