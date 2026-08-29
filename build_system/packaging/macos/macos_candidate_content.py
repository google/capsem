"""Pure staging helpers for the local macOS release candidate."""

from __future__ import annotations

import errno
import json
import os
import shutil
from pathlib import Path

GUEST_PROFILE_ROOT = "file:///Volumes/My%20Shared%20Files/capsem-profiles"


def hardlink_or_copy(source: Path, destination: Path) -> None:
    """Stage immutable bytes cheaply, copying only across filesystems."""

    try:
        os.link(source, destination)
    except OSError as error:
        if error.errno != errno.EXDEV:
            raise
        shutil.copyfile(source, destination)


def stage_candidate_assets(
    manifest_path: Path,
    *,
    source_root: Path,
    destination_root: Path,
) -> Path:
    """Expose exact local assets to Tart without duplicating their bytes."""

    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    assets = manifest.get("assets")
    current = assets.get("current") if isinstance(assets, dict) else None
    releases = assets.get("releases") if isinstance(assets, dict) else None
    release = releases.get(current) if isinstance(releases, dict) else None
    arches = release.get("arches") if isinstance(release, dict) else None
    if not isinstance(current, str) or not isinstance(arches, dict):
        raise RuntimeError("candidate asset manifest has no current architecture cohort")
    release_dir = destination_root / current
    release_dir.mkdir(parents=True)
    for architecture, descriptors in arches.items():
        if not isinstance(architecture, str) or not isinstance(descriptors, dict):
            raise RuntimeError("candidate asset manifest has malformed architecture rows")
        for logical_name, descriptor in descriptors.items():
            source = source_root / architecture / logical_name
            if not source.is_file() or not isinstance(descriptor, dict):
                raise RuntimeError(f"candidate asset is missing: {source}")
            if source.stat().st_size != descriptor.get("size"):
                raise RuntimeError(f"candidate asset size mismatch: {source}")
            hardlink_or_copy(source, release_dir / f"{architecture}-{logical_name}")
    return destination_root


def localize_candidate_profile_urls(manifest_path: Path) -> None:
    """Point generated profile config rows at the Tart profile share."""

    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    rewritten = 0
    profiles = manifest.get("profiles")
    if not isinstance(profiles, dict):
        raise RuntimeError("candidate release manifest has no profiles")
    for profile in profiles.values():
        if not isinstance(profile, dict):
            continue
        architectures = profile.get("architectures")
        if not isinstance(architectures, list):
            continue
        for architecture in architectures:
            if not isinstance(architecture, dict):
                continue
            config = architecture.get("config")
            if not isinstance(config, list):
                continue
            for row in config:
                url = row.get("url") if isinstance(row, dict) else None
                if isinstance(url, str) and url.startswith("/profiles/releases/"):
                    row["url"] = f"{GUEST_PROFILE_ROOT}{url}"
                    rewritten += 1
    if rewritten == 0:
        raise RuntimeError("candidate release manifest has no profile URLs to localize")
    manifest_path.write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
