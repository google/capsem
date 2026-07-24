#!/usr/bin/env python3
"""Resolve the latest serialized source manifest for one release channel."""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.parse import urlparse
from urllib.request import Request, url2pathname, urlopen


USER_AGENT = "capsem-release-source/1"


def source_asset_name(channel: str) -> str:
    if not channel or any(
        not (character.isascii() and (character.isalnum() or character in "-_"))
        for character in channel
    ):
        raise ValueError(f"invalid release channel: {channel!r}")
    return f"channel-source-{channel}.json"


def select_latest_source_asset(
    releases: list[dict[str, Any]], channel: str
) -> dict[str, Any] | None:
    expected = source_asset_name(channel)
    candidates: list[tuple[str, dict[str, Any]]] = []
    for release in releases:
        if release.get("draft") or release.get("prerelease"):
            continue
        timestamp = release.get("published_at") or release.get("created_at")
        if not isinstance(timestamp, str):
            continue
        for asset in release.get("assets", []):
            if isinstance(asset, dict) and asset.get("name") == expected:
                candidates.append((timestamp, asset))
    if not candidates:
        return None
    return max(candidates, key=lambda candidate: candidate[0])[1]


def _read_url(url: str, *, token: str | None = None, api: bool = False) -> bytes:
    parsed = urlparse(url)
    if parsed.scheme == "file":
        return Path(url2pathname(parsed.path)).read_bytes()
    if parsed.scheme not in {"http", "https"}:
        raise ValueError(f"unsupported manifest URL scheme: {parsed.scheme or '<none>'}")
    headers = {"User-Agent": USER_AGENT}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    if api:
        headers["Accept"] = "application/octet-stream"
        headers["X-GitHub-Api-Version"] = "2022-11-28"
    with urlopen(Request(url, headers=headers), timeout=60) as response:
        return response.read()


def _github_releases(repository: str, token: str) -> list[dict[str, Any]]:
    payload = _read_url(
        f"https://api.github.com/repos/{repository}/releases?per_page=100",
        token=token,
    )
    releases = json.loads(payload)
    if not isinstance(releases, list):
        raise ValueError("GitHub releases response is not an array")
    return releases


def validate_source_manifest(payload: bytes, channel: str) -> dict[str, Any]:
    manifest = json.loads(payload)
    if not isinstance(manifest, dict):
        raise ValueError("source manifest must be a JSON object")
    if manifest.get("channel") != channel:
        raise ValueError(
            f"source manifest declares channel {manifest.get('channel')!r}, "
            f"expected {channel!r}"
        )
    if not isinstance(manifest.get("profiles"), dict):
        raise ValueError("source manifest profiles must be an object")
    if not isinstance(manifest.get("packages"), list):
        raise ValueError("source manifest packages must be an array")
    return manifest


def resolve_source_manifest(
    *,
    channel: str,
    repository: str,
    token: str,
    fallback_url: str,
) -> tuple[bytes, str]:
    releases = _github_releases(repository, token)
    asset = select_latest_source_asset(releases, channel)
    if asset is None:
        payload = _read_url(fallback_url)
        source = fallback_url
    else:
        api_url = asset.get("url")
        if not isinstance(api_url, str):
            raise ValueError("selected source-manifest asset has no API URL")
        payload = _read_url(api_url, token=token, api=True)
        source = api_url
    validate_source_manifest(payload, channel)
    return payload, source


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--channel", required=True)
    parser.add_argument(
        "--repository",
        default=os.environ.get("GITHUB_REPOSITORY"),
        required=os.environ.get("GITHUB_REPOSITORY") is None,
    )
    parser.add_argument(
        "--fallback-url",
        help="Public manifest used only before the channel has a serialized source asset.",
    )
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    fallback_url = args.fallback_url or (
        f"https://release.capsem.org/assets/{args.channel}/manifest.json"
    )
    token = os.environ.get("GITHUB_TOKEN", "")
    if not token:
        print("GITHUB_TOKEN is required to resolve source manifests", file=sys.stderr)
        return 1
    try:
        payload, source = resolve_source_manifest(
            channel=args.channel,
            repository=args.repository,
            token=token,
            fallback_url=fallback_url,
        )
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_bytes(payload)
    except (HTTPError, URLError, OSError, ValueError, json.JSONDecodeError) as error:
        print(f"source manifest resolution failed: {error}", file=sys.stderr)
        return 1
    print(f"resolved {args.channel} source manifest from {source}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
