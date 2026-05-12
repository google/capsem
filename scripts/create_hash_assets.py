#!/usr/bin/env python3
"""Create hash-named asset aliases based on the v2 manifest.

Usage: create_hash_assets.py <assets_dir>

Reads manifest.json and, for each arch subdirectory:
  1. Computes the expected set of hash-tagged filenames from the manifest.
  2. Deletes any pre-existing `<stem>-<hex16>(.ext)?` files not in that set --
     those are stale aliases from prior builds whose encoded hash no longer
     matches any manifest entry.
  3. Recreates the expected aliases as hardlinks when possible, falling back
     to copies on filesystems or CI hosts that reject hardlinks.

Cleanup matters because (a) stale names break the content-addressable
naming contract (the hex suffix claims a hash the file no longer has) and
(b) without it, prior builds re-pointed stale names at unrelated inodes on
every run.

Hardlinks share the inode so zero extra disk space is used. The copy fallback
keeps clean Linux CI working when Docker-produced files are not owned by the
runner user and protected-hardlink rules reject `os.link`.
"""

import errno
import json
import os
import re
import shutil
import sys


HASH_TAG_RE = re.compile(r"^(?P<stem>[A-Za-z0-9_]+)-(?P<hex>[0-9a-f]{16})(?P<ext>\.[A-Za-z0-9_.]+)?$")

COPY_FALLBACK_ERRNOS = {
    errno.EACCES,
    errno.EPERM,
    errno.EXDEV,
}

for _name in ("EMLINK", "ENOTSUP", "EOPNOTSUPP"):
    if hasattr(errno, _name):
        COPY_FALLBACK_ERRNOS.add(getattr(errno, _name))


def _expected_hashed_names(manifest: dict) -> dict[str, set[str]]:
    """Map arch -> set of expected hash-tagged filenames across all releases."""
    per_arch: dict[str, set[str]] = {}
    for release in manifest["assets"]["releases"].values():
        for arch_name, assets in release["arches"].items():
            bucket = per_arch.setdefault(arch_name, set())
            for name, entry in assets.items():
                h = entry["hash"][:16]
                dot = name.find(".")
                if dot >= 0:
                    bucket.add(f"{name[:dot]}-{h}{name[dot:]}")
                else:
                    bucket.add(f"{name}-{h}")
    return per_arch


def _cleanup_stale(arch_dir: str, expected: set[str]) -> int:
    """Remove hash-tagged files in arch_dir that aren't in `expected`."""
    removed = 0
    for entry in os.listdir(arch_dir):
        if not HASH_TAG_RE.match(entry):
            continue
        if entry in expected:
            continue
        path = os.path.join(arch_dir, entry)
        if os.path.isfile(path):
            os.unlink(path)
            removed += 1
    return removed


def _link_or_copy(src: str, dst: str) -> str:
    """Create dst as a hardlink to src, or copy when hardlinks are unavailable."""
    try:
        os.link(src, dst)
        return "linked"
    except OSError as exc:
        if exc.errno not in COPY_FALLBACK_ERRNOS:
            raise
        shutil.copy2(src, dst)
        return "copied"


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

    expected_per_arch = _expected_hashed_names(manifest)

    # Sweep each arch dir exactly once for stale aliases, regardless of how
    # many releases reference it in the manifest.
    removed = 0
    for arch_name, expected in expected_per_arch.items():
        arch_dir = os.path.join(assets_dir, arch_name)
        if os.path.isdir(arch_dir):
            removed += _cleanup_stale(arch_dir, expected)

    created = 0
    copied = 0
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
                    mode = _link_or_copy(src, dst)
                    if mode == "copied":
                        copied += 1
                    created += 1

    if removed:
        print(f"  removed {removed} stale hash-tagged alias(es)")
    if created:
        print(f"  created {created} hash-named asset(s)")
    if copied:
        print(f"  copied {copied} hash-named asset(s) because hardlinks were unavailable")


if __name__ == "__main__":
    main()
