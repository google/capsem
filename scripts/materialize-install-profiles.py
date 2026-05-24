#!/usr/bin/env python3
"""Materialize install-time base profiles from a signed asset tree.

The checked-in base profiles are editable drafts. Installers must not seed
their placeholder VM asset declarations verbatim: the service would try to
download from non-existent draft URLs before it ever reaches the local assets
the package already carries.

Usage:
  materialize-install-profiles.py <profile_src_dir> <assets_dir> <out_dir> <asset_source_root>

``asset_source_root`` is either:
  - an absolute path to a local asset root, rendered as file:// URLs, or
  - an http(s) base URL, rendered as <base>/<profile>/<revision>/<arch>/<asset>.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path
from urllib.parse import quote


ASSET_SECTION_RE = re.compile(r"^\[vm\.assets\.([A-Za-z0-9_]+)\.(kernel|initrd|rootfs)\]$")
LOGICAL_NAMES = {
    "kernel": "vmlinuz",
    "initrd": "initrd.img",
    "rootfs": "rootfs.squashfs",
}
CONTENT_TYPES = {
    "kernel": "application/octet-stream",
    "initrd": "application/octet-stream",
    "rootfs": "application/vnd.squashfs",
}


def _usage() -> str:
    return (
        "Usage: materialize-install-profiles.py "
        "<profile_src_dir> <assets_dir> <out_dir> <installed_asset_root>"
    )


def _asset_uri(root: str, profile_id: str, revision: str, arch: str, logical_name: str) -> str:
    if root.startswith(("https://", "http://")):
        base = root.rstrip("/")
        return (
            f"{base}/{quote(profile_id)}/{quote(revision)}/"
            f"{quote(arch)}/{quote(logical_name)}"
        )
    return (Path(root) / arch / logical_name).as_uri()


def _materialized_section(
    arch: str,
    kind: str,
    entry: dict[str, object],
    profile_id: str,
    revision: str,
    asset_source_root: str,
) -> list[str]:
    logical_name = LOGICAL_NAMES[kind]
    hash_hex = entry.get("hash")
    size = entry.get("size")
    if not isinstance(hash_hex, str) or not hash_hex:
        raise ValueError(f"manifest entry for {arch}/{logical_name} has no hash")
    if not isinstance(size, int) or size < 0:
        raise ValueError(f"manifest entry for {arch}/{logical_name} has invalid size")

    asset_url = _asset_uri(asset_source_root, profile_id, revision, arch, logical_name)
    return [
        f"[vm.assets.{arch}.{kind}]",
        f'url = "{asset_url}"',
        f'hash = "blake3:{hash_hex}"',
        f'signature_url = "{asset_url}.minisig"',
        f"size = {size}",
        f'content_type = "{CONTENT_TYPES[kind]}"',
        "",
    ]


def _rewrite_profile(
    source: Path,
    asset_release: str,
    arches: dict[str, dict[str, dict[str, object]]],
    asset_source_root: str,
) -> str:
    lines = source.read_text(encoding="utf-8").splitlines()
    out: list[str] = []
    i = 0
    materialized = 0

    while i < len(lines):
        line = lines[i]
        match = ASSET_SECTION_RE.match(line)
        if not match:
            if line.startswith('revision = "'):
                out.append(f'revision = "{asset_release}"')
            else:
                out.append(line)
            i += 1
            continue

        arch, kind = match.groups()
        logical_name = LOGICAL_NAMES[kind]
        while i + 1 < len(lines) and not lines[i + 1].startswith("["):
            i += 1
        i += 1

        arch_assets = arches.get(arch)
        if arch_assets is None:
            continue
        entry = arch_assets.get(logical_name)
        if entry is None:
            raise ValueError(f"manifest missing {arch}/{logical_name}")

        profile_id = source.name.removesuffix(".profile.toml")
        out.extend(_materialized_section(arch, kind, entry, profile_id, asset_release, asset_source_root))
        materialized += 1

    if materialized == 0:
        raise ValueError(f"{source} did not materialize any VM asset sections")

    rendered = "\n".join(out).rstrip() + "\n"
    if "assets.example.invalid" in rendered:
        raise ValueError(f"{source} still contains assets.example.invalid after rewrite")
    return rendered


def main() -> int:
    if len(sys.argv) != 5:
        print(_usage(), file=sys.stderr)
        return 2

    profile_src_dir = Path(sys.argv[1])
    assets_dir = Path(sys.argv[2])
    out_dir = Path(sys.argv[3])
    asset_source_root = sys.argv[4]
    if not asset_source_root.startswith(("https://", "http://")) and not Path(asset_source_root).is_absolute():
        print("ERROR: asset_source_root must be an absolute path or http(s) URL", file=sys.stderr)
        return 2

    manifest_path = assets_dir / "manifest.json"
    if not manifest_path.is_file():
        print(f"ERROR: manifest missing: {manifest_path}", file=sys.stderr)
        return 1

    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    asset_release = manifest["assets"]["current"]
    arches = manifest["assets"]["releases"][asset_release]["arches"]

    for arch, arch_assets in arches.items():
        for logical_name in ("vmlinuz", "initrd.img", "rootfs.squashfs"):
            if logical_name not in arch_assets:
                raise ValueError(f"manifest missing {arch}/{logical_name}")
            source_asset = assets_dir / arch / logical_name
            if not source_asset.is_file():
                raise ValueError(f"asset file missing: {source_asset}")

    out_dir.mkdir(parents=True, exist_ok=True)
    for profile in sorted(profile_src_dir.glob("*.profile.toml")):
        rendered = _rewrite_profile(profile, asset_release, arches, asset_source_root)
        (out_dir / profile.name).write_text(rendered, encoding="utf-8")
        print(f"  Materialized: {profile.name}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
