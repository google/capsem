"""Select the live manifest graph that a serialized release must preflight."""

from __future__ import annotations

import argparse
import json
import sys
import time
from collections.abc import Callable, Mapping
from pathlib import Path
from typing import Any
from urllib.error import HTTPError
from urllib.parse import urljoin, urlparse
from urllib.request import Request, urlopen

from capsem import release_retirement as retirement

USER_AGENT = "capsem-runtime-preflight/1"
# The catalog read is the first gating step of both release lanes, so a single
# reset connection to the CDN kills an otherwise releasable graph. Retry only
# transport faults and 5xx; a 4xx is an authoritative answer about the catalog
# and must stay fail-closed.
CATALOG_READ_ATTEMPTS = 4
CATALOG_RETRY_BACKOFF_SECONDS = 2.0


def _current_manifest_url(
    catalog: dict[str, Any],
    *,
    release_site: str,
    channel: str,
) -> str:
    channels = catalog.get("channels")
    if not isinstance(channels, dict):
        raise ValueError("public manifest catalog must contain a channels object")
    entry = channels.get(channel)
    if not isinstance(entry, dict):
        raise ValueError(f"public channel {channel} is absent")
    manifests = entry.get("manifests")
    if not isinstance(manifests, list):
        raise ValueError(f"public channel {channel} manifests must be an array")
    current = [
        manifest
        for manifest in manifests
        if isinstance(manifest, dict) and manifest.get("status") == "current"
    ]
    if len(current) != 1:
        raise ValueError(f"public channel {channel} must have exactly one current manifest")
    reference = current[0].get("url")
    if not isinstance(reference, str) or not reference:
        raise ValueError(f"public channel {channel} current manifest URL is missing")
    site = release_site.rstrip("/") + "/"
    resolved = urljoin(site, reference)
    site_url = urlparse(site)
    resolved_url = urlparse(resolved)
    if (
        resolved_url.scheme not in {"http", "https"}
        or resolved_url.scheme != site_url.scheme
        or resolved_url.netloc != site_url.netloc
    ):
        raise ValueError(f"public channel {channel} manifest must remain on the same release site")
    return resolved


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
    channels = catalog.get("channels")
    if not isinstance(channels, dict):
        raise ValueError("public manifest catalog must contain a channels object")
    retired = False
    if channel in channels:
        manifest_channel = channel
        bootstrap = False
    else:
        if not bootstrap_missing_first_party:
            raise ValueError(f"public channel {channel} is absent")
        if channel not in {"stable", "nightly"}:
            raise ValueError("first-party bootstrap requires stable or nightly")
        manifest_channel = "stable" if channel == "nightly" else "nightly"
        if manifest_channel not in channels:
            raise ValueError(f"first-party bootstrap donor {manifest_channel} is absent")
        bootstrap = True
    manifest_url = _current_manifest_url(
        catalog,
        release_site=release_site,
        channel=manifest_channel,
    )
    if bootstrap_missing_first_party and channel in channels:
        selected_channel = retirement.FirstPartyChannel.parse(channel)
        retired = retirement.is_exact_retired_public_graph(
            catalog=catalog,
            channel=selected_channel,
            manifest_url=manifest_url,
            retired=retired_public_graphs or {},
            read_manifest=read_manifest or _read_manifest,
        )
        bootstrap = retired
    return {
        "channel": channel,
        "manifest_channel": manifest_channel,
        "manifest_url": manifest_url,
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


def _write_github_output(path: Path, selection: dict[str, Any]) -> None:
    with path.open("a", encoding="utf-8") as output:
        output.write(f"manifest-url={selection['manifest_url']}\n")
        output.write(f"manifest-channel={selection['manifest_channel']}\n")
        output.write(f"bootstrap={str(selection['bootstrap']).lower()}\n")
        output.write(f"retired={str(selection['retired']).lower()}\n")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--channel", required=True)
    parser.add_argument(
        "--release-site",
        default="https://release.capsem.org",
    )
    parser.add_argument("--bootstrap-missing-first-party", action="store_true")
    parser.add_argument("--github-output", type=Path)
    args = parser.parse_args()
    try:
        selection = select_runtime_preflight_manifest(
            _read_catalog(args.release_site),
            release_site=args.release_site,
            channel=args.channel,
            bootstrap_missing_first_party=args.bootstrap_missing_first_party,
            retired_public_graphs=retirement.load_retired_public_graphs(),
        )
        if args.github_output is not None:
            _write_github_output(args.github_output, selection)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"runtime preflight manifest selection failed: {error}", file=sys.stderr)
        return 1
    print(json.dumps(selection, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
