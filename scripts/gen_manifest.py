#!/usr/bin/env python3
"""Generate manifest.json from B3SUMS + file sizes.

Usage: gen_manifest.py <assets_dir> <cargo_toml_path>

Reads B3SUMS in <assets_dir>, extracts file sizes, reads the workspace
version from Cargo.toml, and writes manifest.json to <assets_dir>.

Produces per-arch nested format when B3SUMS entries have arch prefixes
(e.g., "arm64/vmlinuz"), or flat format for bare filenames.
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
    # Group by arch for per-arch format, or collect flat entries.
    arch_entries: dict[str, list[dict]] = {}
    flat_entries: list[dict] = []

    with open(b3sums_path) as f:
        for line in f:
            parts = line.split(None, 1)
            if len(parts) != 2:
                continue
            h, filepath = parts[0], parts[1].strip()
            full_path = os.path.join(assets_dir, filepath)
            sz = os.path.getsize(full_path) if os.path.exists(full_path) else 0

            if "/" in filepath:
                # Per-arch entry: "arm64/vmlinuz" -> arch="arm64", filename="vmlinuz"
                arch_name, filename = filepath.split("/", 1)
                arch_entries.setdefault(arch_name, []).append(
                    {"filename": filename, "hash": h, "size": sz}
                )
            else:
                flat_entries.append({"filename": filepath, "hash": h, "size": sz})

    # Build release entry: per-arch nested or flat.
    if arch_entries:
        release = {
            arch: {"assets": assets} for arch, assets in arch_entries.items()
        }
    else:
        release = {"assets": flat_entries}

    manifest = {
        "latest": version,
        "releases": {
            version: release,
        },
    }

    manifest_path = os.path.join(assets_dir, "manifest.json")
    with open(manifest_path, "w") as f:
        json.dump(manifest, f, indent=2)
        f.write("\n")

    total = sum(len(v) for v in arch_entries.values()) if arch_entries else len(flat_entries)
    print(f"  manifest.json: {manifest_path} (version {version}, {total} assets)")


if __name__ == "__main__":
    main()
