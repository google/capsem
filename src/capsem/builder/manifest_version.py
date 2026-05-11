"""Helpers for Capsem asset manifest version selection."""

from __future__ import annotations

import datetime
from collections.abc import Mapping
from typing import Any


def asset_date_prefix(day: datetime.date) -> str:
    """Return the YYYY.MMDD asset-version prefix for a date."""
    return day.strftime("%Y.%m%d")


def next_asset_version(
    existing_manifest: Mapping[str, Any] | None,
    *,
    today: datetime.date | None = None,
) -> str:
    """Select the next `YYYY.MMDD.patch` asset version.

    Handles both current v2 manifests (`assets.releases`) and legacy v1
    manifests (`latest`) so local rebuilds and release builds advance the
    same-day patch number consistently.
    """
    today = today or datetime.date.today()
    prefix = asset_date_prefix(today)
    next_patch = 1

    if existing_manifest:
        for version in _asset_versions(existing_manifest):
            if not version.startswith(prefix + "."):
                continue
            try:
                patch = int(version.rsplit(".", 1)[1])
            except (IndexError, ValueError):
                continue
            next_patch = max(next_patch, patch + 1)

    return f"{prefix}.{next_patch}"


def _asset_versions(manifest: Mapping[str, Any]) -> list[str]:
    if manifest.get("format") == 2:
        releases = manifest.get("assets", {}).get("releases", {})
        if isinstance(releases, Mapping):
            return [str(version) for version in releases]

    latest = manifest.get("latest")
    return [str(latest)] if latest else []
