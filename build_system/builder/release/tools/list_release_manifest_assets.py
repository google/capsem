"""List every runtime artifact in a legacy or public release manifest."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any
from urllib.parse import urljoin


def _digest(record: dict[str, Any], label: str) -> tuple[str, int]:
    digest = record.get("digest")
    blake3 = digest.get("blake3") if isinstance(digest, dict) else None
    size = record.get("bytes")
    if not isinstance(blake3, str) or len(blake3) != 64:
        raise ValueError(f"{label} has invalid BLAKE3")
    if not isinstance(size, int) or size < 0:
        raise ValueError(f"{label} has invalid byte size")
    return blake3, size


def _absolute_url(manifest_url: str, value: object, label: str) -> str:
    if not isinstance(value, str) or not value:
        raise ValueError(f"{label} has no URL")
    return urljoin(manifest_url, value)


def _runtime_rows(
    manifest: dict[str, Any], manifest_url: str
) -> list[tuple[str, str, str, str, int, str]]:
    runtime = manifest.get("runtime")
    if not isinstance(runtime, dict):
        raise ValueError("release graph runtime must be an object")
    architectures = runtime.get("architectures")
    if not isinstance(architectures, list) or not architectures:
        raise ValueError("runtime has no architectures")
    rows: dict[str, tuple[str, str, str, str, int, str]] = {}
    for architecture in architectures:
        if not isinstance(architecture, dict):
            raise ValueError("runtime architecture must be an object")
        arch = architecture.get("architecture")
        revision = architecture.get("image_revision")
        if not isinstance(arch, str) or not arch:
            raise ValueError("runtime architecture has no name")
        if not isinstance(revision, str) or not revision:
            raise ValueError(f"runtime/{arch} has no image_revision")
        for section in ("images", "evidence"):
            records = architecture.get(section, [])
            if not isinstance(records, list):
                raise ValueError(f"runtime/{arch} {section} is not a list")
            for index, record in enumerate(records):
                if not isinstance(record, dict):
                    raise ValueError(f"runtime/{arch} {section}[{index}] is not an object")
                label = str(record.get("name") or record.get("kind") or f"{section}-{index}")
                context = f"runtime/{arch}/{label}"
                blake3, size = _digest(record, context)
                url = _absolute_url(manifest_url, record.get("url"), context)
                row = (revision, arch, label, blake3, size, url)
                previous = rows.setdefault(url, row)
                if previous[3:5] != row[3:5]:
                    raise ValueError(
                        f"release graph records disagree on {url}: {previous[3:5]} != {row[3:5]}"
                    )
    if not rows:
        raise ValueError("release graph contains no runtime artifacts")
    return sorted(rows.values(), key=lambda row: (row[1], row[2], row[5]))


def _legacy_rows(
    manifest: dict[str, Any], manifest_url: str
) -> list[tuple[str, str, str, str, int, str]]:
    assets = manifest.get("assets")
    if not isinstance(assets, dict):
        raise ValueError("manifest is neither a release graph nor a legacy asset manifest")
    current = assets.get("current")
    releases = assets.get("releases")
    if not isinstance(current, str) or not isinstance(releases, dict):
        raise ValueError("legacy manifest is missing assets.current/releases")
    release = releases.get(current)
    arches = release.get("arches") if isinstance(release, dict) else None
    if not isinstance(arches, dict) or not arches:
        raise ValueError(f"legacy manifest release {current!r} has no arches")
    asset_base = manifest.get("asset_base") or "/assets/releases"
    if not isinstance(asset_base, str):
        raise ValueError("legacy manifest asset_base must be a string")
    rows = []
    for arch, files in sorted(arches.items()):
        if not isinstance(files, dict):
            raise ValueError(f"legacy manifest arch {arch!r} must be an object")
        for name, record in sorted(files.items()):
            if not isinstance(record, dict):
                raise ValueError(f"legacy manifest asset {arch}/{name} must be an object")
            digest = record.get("hash")
            size = record.get("size")
            if not isinstance(digest, str) or len(digest) != 64:
                raise ValueError(f"legacy manifest asset {arch}/{name} has invalid hash")
            if not isinstance(size, int) or size < 0:
                raise ValueError(f"legacy manifest asset {arch}/{name} has invalid size")
            base = asset_base.rstrip("/")
            version_base = (
                base.replace("{asset_version}", current)
                if "{asset_version}" in base
                else f"{base}/{current}"
            )
            url = urljoin(manifest_url, f"{version_base.rstrip('/')}/{arch}-{name}")
            rows.append((current, arch, name, digest, size, url))
    return rows


def manifest_asset_rows(
    manifest: dict[str, Any], manifest_url: str
) -> list[tuple[str, str, str, str, int, str]]:
    # A channel graph that has published no runtime yet (a binary-only first
    # release) is valid and selects no runtime artifact.
    if isinstance(manifest.get("packages"), list) and manifest.get("runtime") is None:
        return []
    if "runtime" in manifest:
        return _runtime_rows(manifest, manifest_url)
    return _legacy_rows(manifest, manifest_url)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest-path", required=True, type=Path)
    parser.add_argument("--manifest-url", required=True)
    args = parser.parse_args()
    value = json.loads(args.manifest_path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise SystemExit("manifest must contain a JSON object")
    try:
        rows = manifest_asset_rows(value, args.manifest_url)
    except ValueError as error:
        raise SystemExit(str(error)) from error
    for row in rows:
        print("\t".join(str(value) for value in row))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
