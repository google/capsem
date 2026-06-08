#!/usr/bin/env python3
"""Generate v2 manifest.json from B3SUMS + file sizes.

Usage: gen_manifest.py <assets_dir> <cargo_toml_path>

Reads B3SUMS in <assets_dir>, extracts file sizes, reads the workspace
version from Cargo.toml (binary version), derives an asset version from
today's date, and writes a v2 manifest.json to <assets_dir>.

v2 manifest has separate `assets` and `binaries` sections with independent
version tracks and compatibility ranges.
"""

import datetime
import json
import os
import sys


def _same_asset_map(left, right):
    return left == right


def _next_or_existing_asset_version(existing, date_prefix, arch_assets):
    """Reuse the current release for identical assets; otherwise mint a patch."""
    patch = 1
    if not isinstance(existing, dict):
        return f"{date_prefix}.{patch}"
    assets = existing.get("assets", {})
    releases = assets.get("releases", {})
    current = assets.get("current")
    if current in releases:
        current_arches = releases[current].get("arches", {})
        if _same_asset_map(current_arches, arch_assets):
            return current
    for version in releases:
        if not version.startswith(date_prefix + "."):
            continue
        try:
            patch = max(patch, int(version.rsplit(".", 1)[1]) + 1)
        except ValueError:
            continue
    return f"{date_prefix}.{patch}"


def main():
    if len(sys.argv) != 3:
        print(f"Usage: {sys.argv[0]} <assets_dir> <cargo_toml_path>", file=sys.stderr)
        sys.exit(1)

    assets_dir = sys.argv[1]
    cargo_toml = sys.argv[2]

    # Read binary version from Cargo.toml.
    binary_version = None
    with open(cargo_toml) as f:
        for line in f:
            line = line.strip()
            if line.startswith("version") and "=" in line:
                binary_version = line.split("=", 1)[1].strip().strip('"')
                break
    if not binary_version:
        print("ERROR: Could not find version in Cargo.toml", file=sys.stderr)
        sys.exit(1)

    today = datetime.date.today()
    today_str = today.isoformat()

    manifest_path = os.path.join(assets_dir, "manifest.json")
    date_prefix = today.strftime("%Y.%m%d")
    existing_manifest = None
    if os.path.exists(manifest_path):
        try:
            with open(manifest_path) as f:
                existing_manifest = json.load(f)
        except json.JSONDecodeError:
            existing_manifest = None

    # Read B3SUMS and collect entries with file sizes.
    b3sums_path = os.path.join(assets_dir, "B3SUMS")
    # Group by arch: arch -> {logical_name -> {hash, size}}
    arch_assets: dict[str, dict[str, dict]] = {}

    with open(b3sums_path) as f:
        for line in f:
            parts = line.split(None, 1)
            if len(parts) != 2:
                continue
            h, filepath = parts[0], parts[1].strip()
            full_path = os.path.join(assets_dir, filepath)
            sz = os.path.getsize(full_path) if os.path.exists(full_path) else 0

            if "/" in filepath:
                # Per-arch entry: "arm64/vmlinuz" -> arch="arm64", name="vmlinuz"
                arch_name, filename = filepath.split("/", 1)
            else:
                # Flat entry: detect arch from platform or default to "unknown"
                arch_name = "unknown"
                filename = filepath

            arch_assets.setdefault(arch_name, {})[filename] = {
                "hash": h,
                "size": sz,
            }

    asset_version = _next_or_existing_asset_version(
        existing_manifest,
        date_prefix,
        arch_assets,
    )

    manifest = {
        "format": 2,
        "refresh_policy": "24h",
        "assets": {
            "current": asset_version,
            "releases": {
                asset_version: {
                    "date": today_str,
                    "deprecated": False,
                    "min_binary": "1.0.0",
                    "arches": arch_assets,
                },
            },
        },
        "binaries": {
            "current": binary_version,
            "releases": {
                binary_version: {
                    "date": today_str,
                    "deprecated": False,
                    "min_assets": asset_version,
                },
            },
        },
    }

    with open(manifest_path, "w") as f:
        json.dump(manifest, f, indent=2)
        f.write("\n")

    total = sum(len(v) for v in arch_assets.values())
    print(f"  manifest.json: {manifest_path} (assets {asset_version}, binary {binary_version}, {total} assets)")


if __name__ == "__main__":
    main()
