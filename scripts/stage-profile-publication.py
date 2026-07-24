#!/usr/bin/env python3
"""Stage exactly one manifest-described immutable profile publication."""

from __future__ import annotations

import argparse
import json
import shutil
import sys
from pathlib import Path
from urllib.parse import urlparse


def _safe_source(root: Path, relative: str) -> Path:
    candidate = (root / relative).resolve()
    try:
        candidate.relative_to(root.resolve())
    except ValueError as error:
        raise ValueError(f"profile publication source escapes {root}: {relative}") from error
    if not candidate.is_file():
        raise ValueError(f"profile publication source is missing: {candidate}")
    return candidate


def stage_profile_publication(
    manifest_path: Path,
    profile_id: str,
    assets_dir: Path,
    config_root: Path,
    release_dir: Path,
) -> set[Path]:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    profiles = manifest.get("profiles")
    if not isinstance(profiles, dict) or profile_id not in profiles:
        raise ValueError(f"source manifest does not contain profile {profile_id!r}")
    profile = profiles[profile_id]
    if not isinstance(profile, dict):
        raise ValueError(f"source manifest profile {profile_id!r} is malformed")
    if release_dir.exists() and any(release_dir.iterdir()):
        raise ValueError(f"profile publication directory is not empty: {release_dir}")
    release_dir.mkdir(parents=True, exist_ok=True)
    staged: set[Path] = set()
    for architecture in profile.get("architectures", []):
        if not isinstance(architecture, dict):
            raise ValueError(f"profile {profile_id!r} architecture is malformed")
        arch = architecture.get("architecture")
        if not isinstance(arch, str) or not arch:
            raise ValueError(f"profile {profile_id!r} architecture has no name")
        for section in ("config", "images", "evidence"):
            rows = architecture.get(section)
            if not isinstance(rows, list):
                raise ValueError(f"profile {profile_id}/{arch} has no {section} array")
            for row in rows:
                if not isinstance(row, dict):
                    raise ValueError(f"profile {profile_id}/{arch}/{section} row is malformed")
                url = row.get("url")
                if not isinstance(url, str):
                    raise ValueError(f"profile {profile_id}/{arch}/{section} row has no URL")
                destination_name = Path(urlparse(url).path).name
                if not destination_name.startswith(f"{arch}-"):
                    raise ValueError(
                        f"profile publication URL does not encode architecture: {url}"
                    )
                if section == "config":
                    relative = row.get("path")
                    if not isinstance(relative, str):
                        raise ValueError(f"profile {profile_id}/{arch} config row has no path")
                    source = _safe_source(config_root, relative)
                else:
                    source_name = destination_name.removeprefix(f"{arch}-")
                    source = _safe_source(assets_dir / arch, source_name)
                destination = release_dir / destination_name
                if destination in staged:
                    if destination.read_bytes() != source.read_bytes():
                        raise ValueError(
                            f"profile publication has conflicting bytes for {destination_name}"
                        )
                    continue
                shutil.copy2(source, destination)
                staged.add(destination)
    source_name = f"channel-source-{manifest.get('channel')}.json"
    source_destination = release_dir / source_name
    shutil.copy2(manifest_path, source_destination)
    staged.add(source_destination)
    return staged


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--assets-dir", type=Path, required=True)
    parser.add_argument("--config-root", type=Path, default=Path("config"))
    parser.add_argument("--release-dir", type=Path, required=True)
    args = parser.parse_args()
    try:
        staged = stage_profile_publication(
            args.manifest,
            args.profile,
            args.assets_dir,
            args.config_root,
            args.release_dir,
        )
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"profile publication staging failed: {error}", file=sys.stderr)
        return 1
    print(f"staged {len(staged)} immutable profile publication files")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
