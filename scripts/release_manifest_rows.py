#!/usr/bin/env python3
"""Ask a published manifest whether its rows can actually be downloaded.

The check that was missing. `check_package_url` validated the *shape* of a URL
-- host, path template, not-an-asset-tag -- and never fetched it, and nothing
looked at the profile rows at all. So the live stable channel served
`status: current` with HTTP 200 for a month while all three of its package URLs
returned 404, because the tag they pointed at had been deleted after
publication. No build-time check can see that: at build time the bytes were
there. Only asking the live URL can.

A HEAD and a User-Agent. The agent is not optional and is easy to lose:
`release.capsem.org` answers the default `python-urllib` agent with 403, which
reads as a dead row for every site-relative path at once and looks exactly like
the outage this is meant to detect.

Measured rather than assumed, after a ranged GET was carried here on the belief
that HEAD drew false 403s. It does not; that 403 was the agent. HEAD returns 200
and no body from both hosts, while `Range` is honoured by GitHub and ignored by
the site -- so the "cheap" one-byte request pulled the whole 725 KB manifest back
on forty of forty-three rows.
"""

from __future__ import annotations

import urllib.error
import urllib.parse
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from typing import Any

#: Enough of a request to be served rather than filtered.
AGENT = {"User-Agent": "capsem-release-check/1.0"}

#: 200 is the whole answer for a HEAD; 206 only appears if a host insists on
#: answering a body request, which is tolerated rather than expected.
ALIVE = (200, 206)

WORKERS = 8


def manifest_rows(manifest: dict[str, Any], base_url: str) -> list[tuple[str, str]]:
    """Every downloadable thing the manifest names, labelled for a human.

    Packages and profile assets together, because a channel is only usable if
    both resolve -- and the profile half is the half nothing was checking.
    """
    rows: list[tuple[str, str]] = []
    for package in manifest.get("packages") or []:
        if isinstance(package, dict) and (url := package.get("url")):
            rows.append((f"package/{package.get('name', '?')}", url))

    profiles = manifest.get("profiles") or {}
    entries = (
        profiles.items()
        if isinstance(profiles, dict)
        else ((item.get("id", "?"), item) for item in profiles)
    )
    for name, profile in entries:
        if not isinstance(profile, dict):
            continue
        for arch in profile.get("architectures") or []:
            label = f"{name}/{arch.get('architecture', '?')}"
            for group in ("assets", "config"):
                for entry in arch.get(group) or []:
                    if isinstance(entry, dict) and (url := entry.get("url")):
                        rows.append((f"{label}/{entry.get('path', '?')}", url))

    # Site-relative rows are resolved against the manifest they came from, so a
    # relative path is not reported dead for being relative.
    return [(label, urllib.parse.urljoin(base_url, url)) for label, url in rows]


def probe(row: tuple[str, str], timeout: int = 30) -> tuple[str, str, object]:
    """Ask whether the object is there, and report what came back."""
    label, url = row
    try:
        request = urllib.request.Request(url, headers=dict(AGENT), method="HEAD")
        with urllib.request.urlopen(request, timeout=timeout) as response:
            return label, url, response.status
    except urllib.error.HTTPError as error:
        return label, url, error.code
    except Exception as error:  # the failure *is* the finding, so it is returned
        return label, url, repr(error)


def dead_rows(manifest: dict[str, Any], base_url: str) -> list[str]:
    """Which rows could not be downloaded, as failures a caller can report."""
    rows = manifest_rows(manifest, base_url)
    if not rows:
        return ["manifest names no downloadable rows at all"]
    with ThreadPoolExecutor(max_workers=WORKERS) as pool:
        results = list(pool.map(probe, rows))
    return [
        f"{label} is not downloadable ({status}): {url}"
        for label, url, status in results
        if status not in ALIVE
    ]
