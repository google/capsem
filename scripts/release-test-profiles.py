#!/usr/bin/env python3
"""List and validate the exact profile axis for a functional release gate."""

from __future__ import annotations

import argparse
import json
import sys
import tomllib
from pathlib import Path
from typing import Any, cast


def _manifest_profile_ids(manifest: Path) -> list[str] | None:
    try:
        document = json.loads(manifest.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"invalid release test manifest {manifest}: {error}") from error
    if not isinstance(document, dict):
        raise ValueError("release test manifest must be a JSON object")
    profiles = document.get("profiles")
    if profiles is None:
        return None
    if not isinstance(profiles, dict) or not profiles:
        raise ValueError("release test manifest profiles must be a non-empty object")
    selected = []
    for profile_id, profile in sorted(profiles.items()):
        if not isinstance(profile_id, str) or not profile_id:
            raise ValueError("release test manifest has an invalid profile identity")
        if not isinstance(profile, dict):
            raise ValueError(f"release test manifest profile {profile_id} is malformed")
        profile = cast(dict[str, Any], profile)
        if profile.get("status") != "revoked":
            selected.append(profile_id)
    if not selected:
        raise ValueError("release test manifest has no active profiles")
    return selected


def _materialized_profile_ids(profiles_dir: Path) -> list[str]:
    if not profiles_dir.is_dir():
        raise ValueError(f"materialized profile directory is missing: {profiles_dir}")
    selected = []
    for profile_path in sorted(profiles_dir.glob("*/profile.toml")):
        profile_id = profile_path.parent.name
        try:
            document: dict[str, Any] = tomllib.loads(
                profile_path.read_text(encoding="utf-8")
            )
        except (OSError, tomllib.TOMLDecodeError) as error:
            raise ValueError(f"invalid materialized profile {profile_path}: {error}") from error
        if document.get("id") != profile_id:
            raise ValueError(
                f"materialized profile {profile_path} id does not match {profile_id}"
            )
        selected.append(profile_id)
    if not selected:
        raise ValueError(f"no materialized profiles found under {profiles_dir}")
    return selected


def release_test_profiles(profiles_dir: Path, manifest: Path) -> list[str]:
    materialized = _materialized_profile_ids(profiles_dir)
    selected = _manifest_profile_ids(manifest)
    if selected is None:
        selected = materialized
    elif set(materialized) != set(selected):
        raise ValueError(
            "materialized profile catalog does not match the selected manifest: "
            f"manifest={selected}, materialized={materialized}"
        )
    selected.sort(key=lambda profile_id: (profile_id != "code", profile_id))
    return selected


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profiles-dir", required=True, type=Path)
    parser.add_argument("--manifest", required=True, type=Path)
    args = parser.parse_args()
    try:
        profiles = release_test_profiles(args.profiles_dir, args.manifest)
    except ValueError as error:
        print(f"release profile test selection failed: {error}", file=sys.stderr)
        return 1
    print("\n".join(profiles))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
