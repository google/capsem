#!/usr/bin/env python3
"""Fail closed unless an installed Capsem exactly matches its selected release."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import urllib.request
from pathlib import Path
from typing import NoReturn

try:
    from release_glowup import (
        GlowupContractError,
        artifact_identity_from_manifest_package,
    )
except ModuleNotFoundError:
    from scripts.release_glowup import (
        GlowupContractError,
        artifact_identity_from_manifest_package,
    )


METADATA_SCHEMA = "capsem.manifest_metadata.v1"
LEGACY_STATE_PATHS = (
    "manifest-origin.json",
    "update-check.json",
    "update-checks",
    "update-cache",
    "assets/manifest-origin.json",
    "assets/update-check.json",
    "assets/update-checks",
    "assets/update-cache",
)


def fail(message: str) -> NoReturn:
    raise SystemExit(f"installed release verification failed: {message}")


def fetch_bytes(url: str) -> bytes:
    request = urllib.request.Request(url, headers={"User-Agent": "capsem-installed-release-gate"})
    with urllib.request.urlopen(request, timeout=120) as response:
        return response.read()


def positive_integer(metadata: dict[str, object], field: str) -> int:
    value = metadata.get(field)
    if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
        fail(f"manifest-metadata {field} must be a positive integer")
    return value


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--capsem", required=True, type=Path)
    parser.add_argument("--capsem-home", default=Path.home() / ".capsem", type=Path)
    parser.add_argument("--manifest-url", required=True)
    parser.add_argument("--metadata-manifest-url")
    parser.add_argument("--channel", required=True)
    parser.add_argument("--package-version", required=True)
    parser.add_argument("--artifact", type=Path)
    parser.add_argument("--platform")
    parser.add_argument("--architecture")
    parser.add_argument("--evidence-out", type=Path)
    args = parser.parse_args()
    metadata_manifest_url = args.metadata_manifest_url or args.manifest_url
    artifact_options = (args.artifact, args.platform, args.architecture)
    if any(value is not None for value in artifact_options) and not all(
        value is not None for value in artifact_options
    ):
        fail("--artifact, --platform, and --architecture must be supplied together")

    assets_dir = args.capsem_home / "assets"
    installed_manifest_path = assets_dir / "manifest.json"
    metadata_path = assets_dir / "manifest-metadata.json"
    if not args.capsem.is_file():
        fail(f"Capsem CLI is missing: {args.capsem}")
    if not installed_manifest_path.is_file():
        fail(f"installed manifest is missing: {installed_manifest_path}")
    if not metadata_path.is_file():
        fail(f"manifest metadata is missing: {metadata_path}")

    selected_bytes = fetch_bytes(args.manifest_url)
    installed_bytes = installed_manifest_path.read_bytes()
    if installed_bytes != selected_bytes:
        fail("installed manifest is not byte-for-byte identical to the selected manifest URL")
    try:
        manifest = json.loads(installed_bytes)
        metadata = json.loads(metadata_path.read_bytes())
    except json.JSONDecodeError as error:
        fail(f"installed release JSON is invalid: {error}")
    if not isinstance(manifest, dict) or not isinstance(metadata, dict):
        fail("manifest and manifest-metadata must be JSON objects")
    if args.artifact is not None:
        artifact = artifact_identity_from_manifest_package(
            installed_bytes,
            args.artifact,
        )
        expected_identity = {
            "version": args.package_version,
            "platform": args.platform,
            "architecture": args.architecture,
        }
        actual_identity = {
            "version": artifact.version,
            "platform": artifact.platform,
            "architecture": artifact.architecture.value,
        }
        for field, expected in expected_identity.items():
            actual = actual_identity[field]
            if actual != expected:
                fail(
                    f"manifest-selected package {field} is {actual!r}, "
                    f"expected {expected!r}"
                )

    expected_metadata = {
        "schema": METADATA_SCHEMA,
        "manifest_url": metadata_manifest_url,
        "checked_url": args.manifest_url,
        "channel": args.channel,
        "package_version": args.package_version,
        "validation_status": "valid",
    }
    for field, expected in expected_metadata.items():
        if metadata.get(field) != expected:
            fail(
                f"manifest-metadata {field} is {metadata.get(field)!r}, expected {expected!r}"
            )
    if metadata.get("validation_error") is not None:
        fail(f"manifest validation_error is not empty: {metadata['validation_error']!r}")
    if not isinstance(metadata.get("channel_locked"), bool):
        fail("manifest-metadata channel_locked must be boolean")
    if not isinstance(metadata.get("update_available"), bool):
        fail("manifest-metadata update_available must be boolean")
    for field in ("installed_at", "refreshed_at", "checked_at"):
        positive_integer(metadata, field)

    for relative in LEGACY_STATE_PATHS:
        path = args.capsem_home / relative
        if path.exists():
            fail(f"legacy state path still exists: {path}")

    result = subprocess.run(
        [str(args.capsem), "status"],
        check=False,
        capture_output=True,
        text=True,
        timeout=30,
    )
    status = f"{result.stdout}\n{result.stderr}"
    if result.returncode != 0:
        fail(f"capsem status exited {result.returncode}: {status.strip()}")
    for required in (
        "Installed: true",
        "Running:   true",
        "Service:   ok",
        "Gateway:   ok",
        "  status:  valid",
        f"  source:  {metadata_manifest_url}",
    ):
        if required not in status:
            fail(f"capsem status is missing {required!r}")
    version_match = re.search(r"(?m)^Version:\s+(\S+)$", status)
    if version_match is None or version_match.group(1) != args.package_version:
        fail(f"capsem status does not report package version {args.package_version}")
    profile_match = re.search(r"(?m)^Profiles:\s+(\d+)/(\d+) ready\b", status)
    if profile_match is None:
        fail("capsem status has no profile readiness count")
    ready, total = (int(value) for value in profile_match.groups())
    if total <= 0 or ready != total:
        fail(f"profiles are not all ready: {ready}/{total}")
    manifest_profiles = manifest.get("profiles")
    if not isinstance(manifest_profiles, dict) or not manifest_profiles:
        fail("selected release manifest has no profiles")
    if total != len(manifest_profiles):
        fail(
            f"status reports {total} profiles but selected manifest declares "
            f"{len(manifest_profiles)}"
        )
    if args.evidence_out is not None:
        args.evidence_out.parent.mkdir(parents=True, exist_ok=True)
        args.evidence_out.write_text(
            json.dumps(
                {
                    "package_version": args.package_version,
                    "channel": args.channel,
                    "manifest_url": args.manifest_url,
                    "metadata_manifest_url": metadata_manifest_url,
                    "installed": True,
                    "running": True,
                    "service": "ok",
                    "gateway": "ok",
                    "profiles_ready": ready,
                    "profiles_total": total,
                },
                indent=2,
                sort_keys=True,
            )
            + "\n",
            encoding="utf-8",
        )

    print(
        f"verified installed {args.channel} release {args.package_version}: "
        f"{ready}/{total} profiles ready, exact manifest, canonical metadata"
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (GlowupContractError, OSError, subprocess.SubprocessError) as error:
        print(f"installed release verification failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
