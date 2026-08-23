"""Resolve and select the live manifest graph at release trust boundaries."""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
import time
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.parse import urljoin, urlparse
from urllib.request import Request, urlopen

from capsem import release_retirement as retirement
from capsem.release_source_bootstrap import validate_source_manifest

USER_AGENT = "capsem-runtime-preflight/1"
# The catalog read is the first gating step of both release lanes, so a single
# reset connection to the CDN kills an otherwise releasable graph. Retry only
# transport faults and 5xx; a 4xx is an authoritative answer about the catalog
# and must stay fail-closed.
CATALOG_READ_ATTEMPTS = 4
CATALOG_RETRY_BACKOFF_SECONDS = 2.0


class ChannelState(StrEnum):
    """Every possible first-party channel state at a public trust boundary."""

    PUBLISHED = "published"
    ABSENT = "absent"
    RETIRED = "retired"
    UNREACHABLE = "unreachable"
    INVALID = "invalid"


@dataclass(frozen=True)
class ChannelResolution:
    channel: retirement.FirstPartyChannel
    state: ChannelState
    detail: str
    manifest_url: str | None = None
    manifest_sha256: str | None = None

    def as_dict(self) -> dict[str, Any]:
        return {
            "channel": self.channel.value,
            "state": self.state.value,
            "manifest_url": self.manifest_url,
            "manifest_sha256": self.manifest_sha256,
            "detail": self.detail,
        }


def _current_manifest_authority(
    catalog: dict[str, Any],
    *,
    release_site: str,
    channel: retirement.FirstPartyChannel,
) -> tuple[str, str]:
    channels = catalog.get("channels")
    if not isinstance(channels, dict):
        raise ValueError("public manifest catalog must contain a channels object")
    entry = channels.get(channel.value)
    if not isinstance(entry, dict):
        raise ValueError(f"public channel {channel.value} entry must be an object")
    manifests = entry.get("manifests")
    if not isinstance(manifests, list):
        raise ValueError(f"public channel {channel.value} manifests must be an array")
    current = [
        manifest
        for manifest in manifests
        if isinstance(manifest, dict) and manifest.get("status") == "current"
    ]
    if len(current) != 1:
        raise ValueError(f"public channel {channel.value} must have exactly one current manifest")
    reference = current[0].get("url")
    if not isinstance(reference, str) or not reference:
        raise ValueError(f"public channel {channel.value} current manifest URL is missing")
    site = release_site.rstrip("/") + "/"
    resolved = urljoin(site, reference)
    site_url = urlparse(site)
    resolved_url = urlparse(resolved)
    if (
        resolved_url.scheme not in {"http", "https"}
        or resolved_url.scheme != site_url.scheme
        or resolved_url.netloc != site_url.netloc
    ):
        raise ValueError(
            f"public channel {channel.value} manifest must remain on the same release site"
        )
    digest = current[0].get("digest")
    if not isinstance(digest, dict):
        raise ValueError("public current manifest digest must be an object")
    sha256 = digest.get("sha256")
    if not isinstance(sha256, str) or retirement.SHA256_PATTERN.fullmatch(sha256) is None:
        raise ValueError("public current manifest sha256 must be lowercase 64-hex")
    return resolved, sha256


def resolve_channel_state(
    catalog: dict[str, Any],
    *,
    release_site: str,
    channel: retirement.FirstPartyChannel,
    retired_public_graphs: Mapping[retirement.FirstPartyChannel, retirement.RetiredPublicGraph],
    read_manifest: Callable[[str], bytes],
) -> ChannelResolution:
    """Classify one catalog channel after verifying its selected graph bytes."""
    channels = catalog.get("channels")
    if not isinstance(channels, dict):
        return ChannelResolution(
            channel,
            ChannelState.INVALID,
            "public manifest catalog must contain a channels object",
        )
    if channel.value not in channels:
        return ChannelResolution(
            channel,
            ChannelState.ABSENT,
            f"public channel {channel.value} is absent",
        )
    try:
        manifest_url, expected_sha256 = _current_manifest_authority(
            catalog,
            release_site=release_site,
            channel=channel,
        )
    except ValueError as error:
        return ChannelResolution(channel, ChannelState.INVALID, str(error))
    try:
        payload = read_manifest(manifest_url)
    except (HTTPError, URLError, OSError) as error:
        return ChannelResolution(
            channel,
            ChannelState.UNREACHABLE,
            f"public channel {channel.value} manifest is unreachable: {error}",
            manifest_url,
            expected_sha256,
        )
    actual_sha256 = hashlib.sha256(payload).hexdigest()
    if actual_sha256 != expected_sha256:
        return ChannelResolution(
            channel,
            ChannelState.INVALID,
            (
                f"public channel {channel.value} manifest payload digest does not "
                "match its catalog authority"
            ),
            manifest_url,
            expected_sha256,
        )
    try:
        manifest = validate_source_manifest(payload, channel.value)
        if manifest.get("status") != "current":
            raise ValueError("public source manifest status must be current")
    except (UnicodeDecodeError, json.JSONDecodeError, ValueError) as error:
        return ChannelResolution(
            channel,
            ChannelState.INVALID,
            f"public channel {channel.value} manifest is invalid: {error}",
            manifest_url,
            expected_sha256,
        )
    if (
        retirement.retired_public_graph_for_digest(
            channel=channel,
            sha256=expected_sha256,
            retired=retired_public_graphs,
        )
        is not None
    ):
        return ChannelResolution(
            channel,
            ChannelState.RETIRED,
            f"public channel {channel.value} selects an explicitly retired graph",
            manifest_url,
            expected_sha256,
        )
    return ChannelResolution(
        channel,
        ChannelState.PUBLISHED,
        f"public channel {channel.value} selects a verified graph",
        manifest_url,
        expected_sha256,
    )


def select_runtime_preflight_manifest(
    catalog: dict[str, Any],
    *,
    release_site: str,
    channel: str,
    bootstrap_missing_first_party: bool,
    retired_public_graphs: Mapping[retirement.FirstPartyChannel, retirement.RetiredPublicGraph]
    | None = None,
    read_manifest: Callable[[str], bytes] | None = None,
) -> dict[str, Any]:
    selected_channel = retirement.FirstPartyChannel.parse(channel)
    retired_graphs = retired_public_graphs or {}
    reader = read_manifest or _read_manifest
    selected = resolve_channel_state(
        catalog,
        release_site=release_site,
        channel=selected_channel,
        retired_public_graphs=retired_graphs,
        read_manifest=reader,
    )
    retired = selected.state is ChannelState.RETIRED
    if selected.state is ChannelState.PUBLISHED:
        manifest = selected
        manifest_channel = selected_channel
        bootstrap = False
    elif retired:
        if not bootstrap_missing_first_party:
            raise ValueError(selected.detail)
        manifest = selected
        manifest_channel = selected_channel
        bootstrap = True
    elif selected.state is ChannelState.ABSENT:
        if not bootstrap_missing_first_party:
            raise ValueError(selected.detail)
        donor_channel = retirement.other_first_party_channel(selected_channel)
        donor = resolve_channel_state(
            catalog,
            release_site=release_site,
            channel=donor_channel,
            retired_public_graphs=retired_graphs,
            read_manifest=reader,
        )
        if donor.state is not ChannelState.PUBLISHED:
            raise ValueError(
                f"first-party bootstrap donor {donor_channel.value} is {donor.state.value}: "
                f"{donor.detail}"
            )
        manifest = donor
        manifest_channel = donor_channel
        bootstrap = True
    else:
        raise ValueError(selected.detail)
    if manifest.manifest_url is None:
        raise AssertionError("resolved manifest state must carry its URL")
    return {
        "channel": channel,
        "state": selected.state.value,
        "manifest_channel": manifest_channel.value,
        "manifest_url": manifest.manifest_url,
        "manifest_sha256": manifest.manifest_sha256,
        "bootstrap": bootstrap,
        "retired": retired,
    }


def _read_manifest(url: str) -> bytes:
    request = Request(
        url,
        headers={"User-Agent": USER_AGENT, "Cache-Control": "no-cache"},
    )
    with urlopen(request, timeout=60) as response:
        return response.read()


def _read_catalog(release_site: str) -> dict[str, Any]:
    url = release_site.rstrip("/") + "/channels.json"
    request = Request(
        url,
        headers={
            "User-Agent": USER_AGENT,
            "Cache-Control": "no-cache",
            "Pragma": "no-cache",
        },
    )
    payload = _read_catalog_payload(request)
    if not isinstance(payload, dict):
        raise ValueError("public manifest catalog must be a JSON object")
    return payload


def _read_catalog_payload(request: Request) -> Any:
    for attempt in range(1, CATALOG_READ_ATTEMPTS + 1):
        try:
            with urlopen(request, timeout=60) as response:
                return json.load(response)
        except HTTPError as error:
            if error.code < 500 or attempt == CATALOG_READ_ATTEMPTS:
                raise
            reason: object = error
        except OSError as error:
            if attempt == CATALOG_READ_ATTEMPTS:
                raise
            reason = error
        print(
            f"release catalog read attempt {attempt}/{CATALOG_READ_ATTEMPTS} failed: {reason}",
            file=sys.stderr,
        )
        time.sleep(CATALOG_RETRY_BACKOFF_SECONDS * attempt)
    raise AssertionError("unreachable: catalog retry loop must return or raise")


def resolve_remote_channel(
    *,
    release_site: str,
    channel: retirement.FirstPartyChannel,
    retired_public_graphs: Mapping[retirement.FirstPartyChannel, retirement.RetiredPublicGraph]
    | None = None,
    read_catalog: Callable[[str], dict[str, Any]] | None = None,
    read_manifest: Callable[[str], bytes] | None = None,
) -> ChannelResolution:
    """Classify catalog transport as well as the selected manifest graph."""
    try:
        catalog = (read_catalog or _read_catalog)(release_site)
    except (HTTPError, URLError, OSError) as error:
        return ChannelResolution(
            channel,
            ChannelState.UNREACHABLE,
            f"public channel catalog is unreachable: {error}",
        )
    except (UnicodeDecodeError, json.JSONDecodeError, ValueError) as error:
        return ChannelResolution(
            channel,
            ChannelState.INVALID,
            f"public channel catalog is invalid: {error}",
        )
    return resolve_channel_state(
        catalog,
        release_site=release_site,
        channel=channel,
        retired_public_graphs=retired_public_graphs or {},
        read_manifest=read_manifest or _read_manifest,
    )


def _write_github_output(path: Path, selection: Mapping[str, Any]) -> None:
    with path.open("a", encoding="utf-8") as output:
        for key in (
            "state",
            "manifest_url",
            "manifest_sha256",
            "manifest_channel",
            "bootstrap",
            "retired",
        ):
            if key not in selection:
                continue
            value = selection[key]
            if isinstance(value, bool):
                value = str(value).lower()
            output.write(f"{key.replace('_', '-')}={value or ''}\n")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--channel", required=True)
    parser.add_argument(
        "--release-site",
        default="https://release.capsem.org",
    )
    parser.add_argument("--bootstrap-missing-first-party", action="store_true")
    parser.add_argument("--classify-only", action="store_true")
    parser.add_argument("--github-output", type=Path)
    args = parser.parse_args()
    try:
        channel = retirement.FirstPartyChannel.parse(args.channel)
        retired_graphs = retirement.load_retired_public_graphs()
        if args.classify_only:
            resolution = resolve_remote_channel(
                release_site=args.release_site,
                channel=channel,
                retired_public_graphs=retired_graphs,
            )
            selection = resolution.as_dict()
            success = resolution.state not in {
                ChannelState.INVALID,
                ChannelState.UNREACHABLE,
            }
        else:
            selection = select_runtime_preflight_manifest(
                _read_catalog(args.release_site),
                release_site=args.release_site,
                channel=args.channel,
                bootstrap_missing_first_party=args.bootstrap_missing_first_party,
                retired_public_graphs=retired_graphs,
            )
            success = True
        if args.github_output is not None:
            _write_github_output(args.github_output, selection)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"runtime preflight manifest selection failed: {error}", file=sys.stderr)
        return 1
    print(json.dumps(selection, indent=2, sort_keys=True))
    return 0 if success else 1


if __name__ == "__main__":
    raise SystemExit(main())
