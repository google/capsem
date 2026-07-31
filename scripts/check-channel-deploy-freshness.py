#!/usr/bin/env python3
"""Fail if a generated dist would replace an untouched live channel."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path
from urllib.parse import urljoin, urlparse
from urllib.request import Request, urlopen

CHANNELS = ("stable", "nightly")
USER_AGENT = "capsem-channel-deploy/1"


def read_live_manifest(release_site: str, channel: str) -> bytes:
    url = urljoin(
        f"{release_site.rstrip('/')}/",
        f"assets/{channel}/manifest.json",
    )
    with urlopen(
        Request(url, headers={"User-Agent": USER_AGENT}),
        timeout=60,
    ) as response:
        return response.read()


def verify_untouched_channels(
    *,
    selected_channel: str,
    dist: Path,
    release_site: str,
) -> None:
    if selected_channel not in CHANNELS:
        raise ValueError(f"selected channel must be stable or nightly: {selected_channel}")
    parsed = urlparse(release_site)
    if parsed.scheme not in {"http", "https"} or not parsed.netloc:
        raise ValueError("release site must be an absolute HTTP(S) URL")
    for channel in CHANNELS:
        if channel == selected_channel:
            continue
        candidate_path = dist / "assets" / channel / "manifest.json"
        if not candidate_path.is_file():
            raise ValueError(f"generated dist is missing untouched {channel} manifest")
        candidate = candidate_path.read_bytes()
        live = read_live_manifest(release_site, channel)
        if candidate != live:
            raise ValueError(
                f"untouched {channel} manifest changed after candidate assembly; "
                "refusing to replace another channel"
            )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--selected-channel", required=True)
    parser.add_argument("--dist", type=Path, required=True)
    parser.add_argument("--release-site", required=True)
    args = parser.parse_args()
    try:
        verify_untouched_channels(
            selected_channel=args.selected_channel,
            dist=args.dist,
            release_site=args.release_site,
        )
    except (OSError, ValueError) as error:
        print(f"channel deploy freshness check failed: {error}", file=sys.stderr)
        return 1
    print("untouched public channel manifests remain byte-identical")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
