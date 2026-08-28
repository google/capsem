#!/usr/bin/env python3
"""Inspect and verify the exact package cohort selected by a release manifest."""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
import urllib.request
from pathlib import Path
from typing import Any

import blake3

USER_AGENT = {"User-Agent": "capsem-package-storage/1"}


def load_manifest(path: Path) -> dict[str, Any]:
    document = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(document, dict):
        raise ValueError("release manifest must be a JSON object")
    return document


def current_packages(document: dict[str, Any]) -> list[dict[str, Any]]:
    packages = document.get("packages")
    if not isinstance(packages, list):
        raise ValueError("release manifest packages must be an array")
    if any(not isinstance(row, dict) for row in packages):
        raise ValueError("every release package must be an object")
    return [row for row in packages if row.get("status") == "current"]


def selected_version(document: dict[str, Any], platform: str, architecture: str) -> str:
    matches = [
        row
        for row in current_packages(document)
        if row.get("platform") == platform and row.get("architecture") == architecture
    ]
    if len(matches) != 1 or not isinstance(matches[0].get("version"), str):
        raise ValueError(
            f"manifest must contain exactly one versioned current {platform}/{architecture} package"
        )
    return matches[0]["version"]


def fetch(url: str) -> bytes:
    request = urllib.request.Request(url, headers=dict(USER_AGENT))
    with urllib.request.urlopen(request, timeout=120) as response:
        return response.read()


def _field(row: dict[str, Any], name: str) -> str:
    value = row.get(name)
    if not isinstance(value, str) or not value:
        raise ValueError(f"current package has no {name}")
    return value


def verify_storage(
    document: dict[str, Any],
    *,
    expected_prefix: str,
    expected_version: str,
    expected_count: int,
    work_dir: Path,
) -> int:
    packages = current_packages(document)
    if len(packages) != expected_count:
        raise ValueError(
            f"manifest selects {len(packages)} current packages, expected {expected_count}"
        )
    work_dir.mkdir(parents=True, exist_ok=True)
    for package in packages:
        name = _field(package, "name")
        if Path(name).name != name:
            raise ValueError(f"package name is not a basename: {name!r}")
        version = _field(package, "version")
        if version != expected_version:
            raise ValueError(f"{name} version is {version!r}, expected {expected_version!r}")
        url = _field(package, "url")
        if not url.startswith(expected_prefix):
            raise ValueError(f"{name} URL is outside {expected_prefix}: {url}")
        digest = package.get("digest")
        if not isinstance(digest, dict):
            raise ValueError(f"{name} has no digest object")
        expected_sha256 = _field(digest, "sha256")
        expected_blake3 = _field(digest, "blake3")
        payload = fetch(url)
        (work_dir / name).write_bytes(payload)
        actual_sha256 = hashlib.sha256(payload).hexdigest()
        actual_blake3 = blake3.blake3(payload).hexdigest()
        if actual_sha256 != expected_sha256:
            raise ValueError(f"{name} SHA-256 is {actual_sha256}, expected {expected_sha256}")
        if actual_blake3 != expected_blake3:
            raise ValueError(f"{name} BLAKE3 is {actual_blake3}, expected {expected_blake3}")
    return len(packages)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    version = commands.add_parser("selected-version")
    version.add_argument("--manifest", type=Path, required=True)
    version.add_argument("--platform", required=True)
    version.add_argument("--architecture", required=True)
    storage = commands.add_parser("verify-storage")
    storage.add_argument("--manifest", type=Path, required=True)
    storage.add_argument("--expected-prefix", required=True)
    storage.add_argument("--expected-version", required=True)
    storage.add_argument("--expected-count", type=int, default=3)
    storage.add_argument("--work-dir", type=Path, required=True)
    args = parser.parse_args()
    try:
        document = load_manifest(args.manifest)
        if args.command == "selected-version":
            print(selected_version(document, args.platform, args.architecture))
        else:
            count = verify_storage(
                document,
                expected_prefix=args.expected_prefix,
                expected_version=args.expected_version,
                expected_count=args.expected_count,
                work_dir=args.work_dir,
            )
            print(f"verified {count} current release packages")
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"release package verification failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
