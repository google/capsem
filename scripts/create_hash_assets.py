#!/usr/bin/env python3
"""Create hash-named hardlinks for asset files based on the v2 manifest.

Usage: create_hash_assets.py <assets_dir>

Reads manifest.json, creates {stem}-{hash16}.{ext} hardlinks to each asset
file in the per-arch subdirectories. Hardlinks share the inode so zero extra
disk space is used.
"""

import json
import os
import sys


def main():
    if len(sys.argv) != 2:
        print(f"Usage: {sys.argv[0]} <assets_dir>", file=sys.stderr)
        sys.exit(1)

    assets_dir = sys.argv[1]
    manifest_path = os.path.join(assets_dir, "manifest.json")

    with open(manifest_path) as f:
        manifest = json.load(f)

    if manifest.get("format") != 2:
        print("Not a v2 manifest, skipping hash asset creation", file=sys.stderr)
        return

    created = 0
    for release in manifest["assets"]["releases"].values():
        for arch_name, assets in release["arches"].items():
            arch_dir = os.path.join(assets_dir, arch_name)
            if not os.path.isdir(arch_dir):
                continue
            for name, entry in assets.items():
                h = entry["hash"][:16]
                dot = name.find(".")
                if dot >= 0:
                    hashed = f"{name[:dot]}-{h}{name[dot:]}"
                else:
                    hashed = f"{name}-{h}"
                src = os.path.join(arch_dir, name)
                dst = os.path.join(arch_dir, hashed)
                if os.path.exists(src):
                    if os.path.exists(dst):
                        os.unlink(dst)
                    os.link(src, dst)
                    created += 1

    if created:
        print(f"  created {created} hash-named asset(s)")


if __name__ == "__main__":
    main()
