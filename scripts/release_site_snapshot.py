#!/usr/bin/env python3
"""Exact served-byte snapshots for preview activation and rollback proof."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
from typing import Any
from urllib.parse import quote, urljoin, urlparse

CLOUDFLARE_CONTROL_FILES = frozenset({"_headers", "_redirects", "_routes.json"})


def _normalize_release_site(release_site: str) -> str:
    parsed = urlparse(release_site)
    return release_site if parsed.scheme else Path(release_site).resolve().as_uri()


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
    checker: Any, release_site: str, *, dist: Path | None = None
) -> dict[str, object]:
    """Bind every body consumed by the validator to a location-independent path."""

    if dist is not None:
        _fetch_complete_distribution(checker, release_site, dist)
    cache = getattr(checker, "_FETCH_BYTES_CACHE", None)
    if not isinstance(cache, dict) or not cache:
        raise RuntimeError("release validator produced no fetched-byte evidence")
    site = urlparse(_normalize_release_site(release_site))
    entries: dict[str, dict[str, int | str]] = {}
    for url, fetched in sorted(cache.items()):
        body = getattr(fetched, "data", None)
        error = getattr(fetched, "error", None)
        if not isinstance(url, str) or not isinstance(body, bytes) or error is not None:
            continue
        parsed = urlparse(url)
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
