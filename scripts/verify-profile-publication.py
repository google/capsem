#!/usr/bin/env python3
"""Verify one immutable profile release directory against its source manifest."""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path
from urllib.parse import urlparse

import blake3


def verify_profile_publication(
    manifest_path: Path,
    profile_id: str,
    publication_base: str,
    release_dir: Path,
) -> set[Path]:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    profiles = manifest.get("profiles")
    if not isinstance(profiles, dict) or profile_id not in profiles:
        raise ValueError(f"source manifest does not contain profile {profile_id!r}")
    profile = profiles[profile_id]
    if not isinstance(profile, dict):
        raise ValueError(f"source manifest profile {profile_id!r} is malformed")
    channel = manifest.get("channel")
    revision = profile.get("revision")
    if not isinstance(channel, str) or not channel:
        raise ValueError("source manifest has no channel")
    if profile.get("id") != profile_id:
        raise ValueError(f"source manifest profile key does not match {profile_id!r}")
    if not isinstance(revision, str) or not revision:
        raise ValueError(f"source manifest profile {profile_id!r} has no revision")
    parsed = urlparse(publication_base)
    identity = parsed.path.rstrip("/").rsplit("/", maxsplit=1)[-1]
    expected_identity = f"profile-{channel}-{profile_id}-{revision}"
    if identity != expected_identity:
        raise ValueError(
            "immutable profile publication base does not match the "
            f"channel/profile/revision identity {expected_identity}"
        )
    base = publication_base.rstrip("/") + "/"
    expected: set[Path] = set()
    seen_urls: dict[str, tuple[int, str, str]] = {}
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
                if not isinstance(url, str) or not url.startswith(base):
                    raise ValueError(
                        f"profile {profile_id}/{arch}/{section} URL is outside "
                        f"its immutable publication: {url!r}"
                    )
                expected_bytes = row.get("bytes")
                digest = row.get("digest")
                if not isinstance(expected_bytes, int) or not isinstance(digest, dict):
                    raise ValueError(f"immutable profile artifact metadata mismatch: {url}")
                identity = (
                    expected_bytes,
                    str(digest.get("sha256")),
                    str(digest.get("blake3")),
                )
                previous_identity = seen_urls.setdefault(url, identity)
                if previous_identity != identity:
                    raise ValueError(
                        f"duplicate immutable profile URL has conflicting metadata: {url}"
                    )
                name = url.removeprefix(base)
                if (
                    not name
                    or "/" in name
                    or Path(name).name != name
                    or not name.startswith(f"{arch}-")
                ):
                    raise ValueError(f"invalid immutable profile artifact name: {name!r}")
                path = release_dir / name
                if not path.is_file():
                    raise ValueError(f"immutable profile artifact is missing: {path}")
                payload = path.read_bytes()
                if expected_bytes != len(payload):
                    raise ValueError(f"immutable profile artifact metadata mismatch: {name}")
                if digest.get("sha256") != hashlib.sha256(payload).hexdigest():
                    raise ValueError(f"immutable profile artifact SHA-256 mismatch: {name}")
                if digest.get("blake3") != blake3.blake3(payload).hexdigest():
                    raise ValueError(f"immutable profile artifact BLAKE3 mismatch: {name}")
                expected.add(path)
        software = architecture.get("software")
        if not isinstance(software, list):
            raise ValueError(f"profile {profile_id}/{arch} has no software array")
        if software:
            inventory_urls = [
                row.get("url")
                for row in architecture["evidence"]
                if isinstance(row, dict) and row.get("kind") == "software_inventory"
            ]
            if len(inventory_urls) != 1 or not isinstance(inventory_urls[0], str):
                raise ValueError(
                    f"profile {profile_id}/{arch} software evidence must have exactly "
                    "one manifest-owned software_inventory URL"
                )
            inventory_url = inventory_urls[0]
            for row in software:
                if not isinstance(row, dict) or row.get("evidence") != inventory_url:
                    raise ValueError(
                        f"profile {profile_id}/{arch} software evidence does not match "
                        f"its manifest-owned software_inventory URL: {row!r}"
                    )
    if not expected:
        raise ValueError(f"profile {profile_id!r} contains no immutable artifacts")
    source_name = f"channel-source-{manifest.get('channel')}.json"
    allowed = expected | {release_dir / source_name}
    actual = {path for path in release_dir.iterdir() if path.is_file()}
    if actual != allowed:
        extra = sorted(str(path) for path in actual - allowed)
        missing = sorted(str(path) for path in allowed - actual)
        raise ValueError(
            f"immutable profile release file set mismatch: extra={extra}, missing={missing}"
        )
    if parsed.scheme != "https" or not parsed.netloc:
        raise ValueError("immutable profile publication base must be HTTPS")
    return expected


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--publication-base", required=True)
    parser.add_argument("--release-dir", type=Path, required=True)
    args = parser.parse_args()
    try:
        verified = verify_profile_publication(
            args.manifest,
            args.profile,
            args.publication_base,
            args.release_dir,
        )
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"profile publication verification failed: {error}", file=sys.stderr)
        return 1
    print(f"verified {len(verified)} immutable profile artifacts")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
