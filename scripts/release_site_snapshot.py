#!/usr/bin/env python3
"""Exact served-byte snapshots for preview activation and rollback proof."""

from __future__ import annotations

import hashlib
import json
import time
from collections.abc import Callable
from pathlib import Path
from typing import Any
from urllib.parse import quote, urljoin, urlparse

CLOUDFLARE_CONTROL_FILES = frozenset({"_headers", "_redirects", "_routes.json"})


def _normalize_release_site(release_site: str) -> str:
    parsed = urlparse(release_site)
    return release_site if parsed.scheme else Path(release_site).resolve().as_uri()


def retain_successful_external_fetches(checker: Any, release_site: str) -> None:
    """Refetch mutable site bytes without redownloading immutable graph bytes."""

    cache = getattr(checker, "_FETCH_BYTES_CACHE", None)
    if not isinstance(cache, dict):
        return
    site = urlparse(_normalize_release_site(release_site))
    retained: dict[str, Any] = {}
    for url, fetched in cache.items():
        if not isinstance(url, str):
            continue
        parsed = urlparse(url)
        same_origin = (parsed.scheme, parsed.netloc) == (site.scheme, site.netloc)
        body = getattr(fetched, "data", None)
        error = getattr(fetched, "error", None)
        if not same_origin and isinstance(body, bytes) and error is None:
            retained[url] = fetched
    cache.clear()
    cache.update(retained)


def _fetch_complete_distribution(checker: Any, release_site: str, dist: Path) -> None:
    fetch = getattr(checker, "fetch_bytes", None)
    if not callable(fetch):
        raise RuntimeError("release validator does not expose its byte fetcher")
    site = _normalize_release_site(release_site).rstrip("/") + "/"
    for path in sorted(candidate for candidate in dist.rglob("*") if candidate.is_file()):
        relative = path.relative_to(dist).as_posix()
        if relative in CLOUDFLARE_CONTROL_FILES:
            continue
        url = urljoin(site, quote(relative))
        fetched = fetch(url)
        body = getattr(fetched, "data", None)
        error = getattr(fetched, "error", None)
        if error is not None:
            raise RuntimeError(f"complete distribution fetch failed for /{relative}: {error}")
        if not isinstance(body, bytes) or body != path.read_bytes():
            raise RuntimeError(f"served bytes differ from deploy root for /{relative}")


def release_fetch_snapshot(
    checker: Any,
    release_site: str,
    *,
    dist: Path | None = None,
    include_dist_in_snapshot: bool = True,
    same_origin_only: bool = False,
) -> dict[str, object]:
    """Bind every body consumed by the validator to a location-independent path."""

    cache = getattr(checker, "_FETCH_BYTES_CACHE", None)
    contract_urls = (
        set(cache)
        if dist is not None and not include_dist_in_snapshot and isinstance(cache, dict)
        else None
    )
    if dist is not None:
        _fetch_complete_distribution(checker, release_site, dist)
    cache = getattr(checker, "_FETCH_BYTES_CACHE", None)
    if not isinstance(cache, dict) or not cache:
        raise RuntimeError("release validator produced no fetched-byte evidence")
    site = urlparse(_normalize_release_site(release_site))
    entries: dict[str, dict[str, int | str]] = {}
    for url, fetched in sorted(cache.items()):
        if not isinstance(url, str):
            continue
        if contract_urls is not None and url not in contract_urls:
            continue
        parsed = urlparse(url)
        if same_origin_only and (parsed.scheme, parsed.netloc) != (site.scheme, site.netloc):
            continue
        body = getattr(fetched, "data", None)
        error = getattr(fetched, "error", None)
        if same_origin_only and (error is not None or not isinstance(body, bytes)):
            raise RuntimeError(f"same-origin snapshot fetch failed for {url}: {error}")
        if not isinstance(body, bytes) or error is not None:
            continue
        if (parsed.scheme, parsed.netloc) == (site.scheme, site.netloc):
            key = parsed.path or "/"
            if site.scheme == "file":
                try:
                    key = "/" + Path(parsed.path).relative_to(Path(site.path)).as_posix()
                except ValueError:
                    key = url
            if parsed.query:
                key = f"{key}?{parsed.query}"
        else:
            key = url
        identity: dict[str, int | str] = {
            "bytes": len(body),
            "sha256": hashlib.sha256(body).hexdigest(),
        }
        if key in entries and entries[key] != identity:
            raise RuntimeError(f"release validator fetched conflicting bodies for {key}")
        entries[key] = identity
    if "/channels.json" not in entries:
        raise RuntimeError("release snapshot omitted /channels.json")
    if not any(key.startswith("/assets/") and key.endswith("/manifest.json") for key in entries):
        raise RuntimeError("release snapshot omitted every channel manifest")
    return {"schema": "capsem.release_site_snapshot.v1", "entries": entries}


def write_snapshot(path: Path, snapshot: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    pending = path.with_name(f".{path.name}.tmp")
    pending.write_text(json.dumps(snapshot, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    pending.replace(path)


def require_snapshot(path: Path, actual: dict[str, object]) -> None:
    expected = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(expected, dict) or expected.get("schema") != actual["schema"]:
        raise RuntimeError(f"unsupported prior release snapshot: {path}")
    expected_entries = expected.get("entries")
    actual_entries = actual["entries"]
    if not isinstance(expected_entries, dict) or not isinstance(actual_entries, dict):
        raise RuntimeError("release snapshot entries must be objects")
    expected_map = {str(key): value for key, value in expected_entries.items()}
    actual_map = {str(key): value for key, value in actual_entries.items()}
    missing = sorted(set(expected_map) - set(actual_map))
    extra = sorted(set(actual_map) - set(expected_map))
    changed = sorted(
        key for key in set(expected_map) & set(actual_map) if expected_map[key] != actual_map[key]
    )
    if missing or extra or changed:
        raise RuntimeError(
            f"served distribution differs from snapshot: missing={missing}, "
            f"extra={extra}, changed={changed}"
        )


def snapshot_distribution_bytes(
    checker: Any,
    release_site: str,
    *,
    populate: Callable[[], object],
    attempts: int,
    delay_seconds: float,
    snapshot_out: Path | None,
    expect_snapshot: Path | None,
    require_valid: bool = False,
    same_origin_only: bool = True,
    dist: Path | None = None,
    include_dist_in_snapshot: bool = True,
) -> None:
    """Retry validation and exact bytes as one propagation boundary."""
    last_error: OSError | RuntimeError | ValueError | None = None
    rounds = max(attempts, 1)
    for attempt in range(1, rounds + 1):
        retain_successful_external_fetches(checker, release_site)
        try:
            result = populate()
            if require_valid and result != 0:
                raise RuntimeError("release contract validation failed")
            snapshot = release_fetch_snapshot(
                checker,
                release_site,
                dist=dist,
                include_dist_in_snapshot=include_dist_in_snapshot,
                same_origin_only=same_origin_only,
            )
            if snapshot_out is not None:
                write_snapshot(snapshot_out, snapshot)
            elif expect_snapshot is not None:
                require_snapshot(expect_snapshot, snapshot)
            return
        except (OSError, RuntimeError, ValueError) as error:
            last_error = error
        if attempt != rounds:
            time.sleep(delay_seconds)
    raise RuntimeError(f"release-channel byte snapshot failed: {last_error}")
