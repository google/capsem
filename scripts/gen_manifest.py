#!/usr/bin/env python3
"""Generate manifest.json from B3SUMS + file sizes.

Usage: gen_manifest.py <assets_dir> <cargo_toml_path>

Reads B3SUMS in <assets_dir>, extracts file sizes, reads the workspace
version from Cargo.toml, and writes manifest.json to <assets_dir>.
"""

import json
import os
import sys


def main():
    if len(sys.argv) != 3:
        print(f"Usage: {sys.argv[0]} <assets_dir> <cargo_toml_path>", file=sys.stderr)
        sys.exit(1)

    assets_dir = sys.argv[1]
    cargo_toml = sys.argv[2]

    # Read version from Cargo.toml.
    version = None
    with open(cargo_toml) as f:
        for line in f:
            line = line.strip()
            if line.startswith("version") and "=" in line:
                version = line.split("=", 1)[1].strip().strip('"')
                break
    if not version:
        print("ERROR: Could not find version in Cargo.toml", file=sys.stderr)
        sys.exit(1)

    # Read B3SUMS and collect entries with file sizes.
    b3sums_path = os.path.join(assets_dir, "B3SUMS")
    entries = []
    with open(b3sums_path) as f:
        for line in f:
            parts = line.split(None, 1)
            if len(parts) == 2:
                h, filename = parts[0], parts[1].strip()
                filepath = os.path.join(assets_dir, filename)
                sz = os.path.getsize(filepath) if os.path.exists(filepath) else 0
                entries.append({"filename": filename, "hash": h, "size": sz})

    manifest = {
        "latest": version,
        "releases": {
            version: {"assets": entries},
        },
    }

    manifest_path = os.path.join(assets_dir, "manifest.json")
    with open(manifest_path, "w") as f:
        json.dump(manifest, f, indent=2)
        f.write("\n")

    print(f"  manifest.json: {manifest_path} (version {version}, {len(entries)} assets)")


if __name__ == "__main__":
    main()
