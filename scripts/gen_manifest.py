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

    # Derive asset version: YYYY.MMDD.patch
    # Check existing manifest for same-day releases to increment patch.
    manifest_path = os.path.join(assets_dir, "manifest.json")
    date_prefix = today.strftime("%Y.%m%d")
    patch = 1
    if os.path.exists(manifest_path):
        try:
            with open(manifest_path) as f:
                existing = json.load(f)
            # v2 format
            if existing.get("format") == 2:
                for v in existing.get("assets", {}).get("releases", {}):
                    if v.startswith(date_prefix + "."):
                        p = int(v.rsplit(".", 1)[1])
                        patch = max(patch, p + 1)
            # v1 format -- check if latest matches today's date pattern
            elif "latest" in existing:
                v = existing["latest"]
                if v.startswith(date_prefix + "."):
                    p = int(v.rsplit(".", 1)[1])
                    patch = max(patch, p + 1)
        except (json.JSONDecodeError, ValueError, KeyError):
            pass

    asset_version = f"{date_prefix}.{patch}"

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

    manifest = {
        "format": 2,
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
