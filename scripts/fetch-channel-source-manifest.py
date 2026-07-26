#!/usr/bin/env python3
"""Resolve the latest serialized source manifest for one release channel."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any, Callable
from urllib.error import HTTPError, URLError
from urllib.parse import urlparse
from urllib.request import Request, url2pathname, urlopen


USER_AGENT = "capsem-release-source/1"
ROOT = Path(__file__).resolve().parents[1]


class ChannelSourceUnavailable(RuntimeError):
    """No serialized source asset or valid public channel exists yet."""


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
    candidates: list[tuple[str, int, dict[str, Any]]] = []
    for release in releases:
        if release.get("draft") or release.get("prerelease"):
            continue
        release_timestamp = release.get("published_at") or release.get(
            "created_at"
        )
        if not isinstance(release_timestamp, str):
            continue
        for asset in release.get("assets", []):
            if isinstance(asset, dict) and asset.get("name") == expected:
                asset_timestamp = asset.get("updated_at") or asset.get(
                    "created_at"
                )
                timestamp = (
                    asset_timestamp
                    if isinstance(asset_timestamp, str)
                    else release_timestamp
                )
                asset_id = asset.get("id")
                candidates.append(
                    (
                        timestamp,
                        asset_id if isinstance(asset_id, int) else -1,
                        asset,
                    )
                )
    if not candidates:
        return None
    return max(candidates, key=lambda candidate: candidate[:2])[2]


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
    releases: list[dict[str, Any]] = []
    page = 1
    while True:
        payload = _read_url(
            f"https://api.github.com/repos/{repository}/releases"
            f"?per_page=100&page={page}",
            token=token,
        )
        rows = json.loads(payload)
        if not isinstance(rows, list):
            raise ValueError(f"GitHub releases page {page} is not an array")
        releases.extend(row for row in rows if isinstance(row, dict))
        if len(rows) < 100:
            return releases
        page += 1


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


def public_channel_is_absent(payload: bytes, channel: str) -> bool:
    catalog = json.loads(payload)
    if not isinstance(catalog, dict) or not isinstance(catalog.get("channels"), dict):
        raise ValueError("public channel catalog must contain a channels object")
    return channel not in catalog["channels"]


def bootstrap_source_manifest(
    *,
    channel: str,
    profile: str,
    donor_payload: bytes,
    output: Path,
    runner: Callable[..., subprocess.CompletedProcess[str]] = subprocess.run,
) -> bytes:
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(
        dir=output.parent,
        prefix=f".{channel}-bootstrap-donor-",
        suffix=".json",
        delete=False,
    ) as handle:
        handle.write(donor_payload)
        donor_path = Path(handle.name)
    try:
        runner(
            [
                "cargo",
                "run",
                "-p",
                "capsem-admin",
                "--",
                "release",
                "--channel",
                channel,
                "--profile",
                profile,
                "--bootstrap-from-manifest",
                str(donor_path),
                "--bootstrap-output",
                str(output),
                "--json",
            ],
            cwd=ROOT,
            check=True,
            text=True,
        )
        payload = output.read_bytes()
        validate_source_manifest(payload, channel)
        return payload
    finally:
        donor_path.unlink(missing_ok=True)


def _other_first_party_channel(channel: str) -> str:
    if channel == "stable":
        return "nightly"
    if channel == "nightly":
        return "stable"
    raise ValueError("first-party bootstrap requires stable or nightly")


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
        try:
            payload = _read_url(fallback_url)
            validate_source_manifest(payload, channel)
        except (
            HTTPError,
            URLError,
            OSError,
            ValueError,
            json.JSONDecodeError,
        ) as error:
            raise ChannelSourceUnavailable(
                f"no valid serialized or public source exists for {channel}"
            ) from error
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
    parser.add_argument(
        "--bootstrap-missing-first-party",
        action="store_true",
        help="Initialize an absent stable/nightly source through capsem-admin.",
    )
    parser.add_argument(
        "--profile",
        help="Selected profile required when bootstrapping an absent first-party channel.",
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
        try:
            payload, source = resolve_source_manifest(
                channel=args.channel,
                repository=args.repository,
                token=token,
                fallback_url=fallback_url,
            )
        except ChannelSourceUnavailable as error:
            if not args.bootstrap_missing_first_party:
                raise
            if not args.profile:
                raise ValueError(
                    "--profile is required with --bootstrap-missing-first-party"
                ) from error
            parsed_fallback = urlparse(fallback_url)
            if parsed_fallback.scheme not in {"http", "https"} or not parsed_fallback.netloc:
                raise ValueError(
                    "first-party bootstrap requires an HTTP release-site fallback"
                ) from error
            catalog_url = (
                f"{parsed_fallback.scheme}://{parsed_fallback.netloc}/channels.json"
            )
            if not public_channel_is_absent(_read_url(catalog_url), args.channel):
                raise ValueError(
                    f"public channel {args.channel} exists but its source manifest is invalid"
                ) from error
            donor_channel = _other_first_party_channel(args.channel)
            donor_payload, donor_source = resolve_source_manifest(
                channel=donor_channel,
                repository=args.repository,
                token=token,
                fallback_url=(
                    f"{parsed_fallback.scheme}://{parsed_fallback.netloc}"
                    f"/assets/{donor_channel}/manifest.json"
                ),
            )
            payload = bootstrap_source_manifest(
                channel=args.channel,
                profile=args.profile,
                donor_payload=donor_payload,
                output=args.output,
            )
            source = f"capsem-admin bootstrap from {donor_source}"
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_bytes(payload)
    except (
        ChannelSourceUnavailable,
        HTTPError,
        URLError,
        OSError,
        subprocess.CalledProcessError,
        ValueError,
        json.JSONDecodeError,
    ) as error:
        print(f"source manifest resolution failed: {error}", file=sys.stderr)
        return 1
    print(f"resolved {args.channel} source manifest from {source}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
