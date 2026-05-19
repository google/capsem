"""Shared Profile V2 asset-backed E2E fixture helpers."""

from __future__ import annotations

import json
import os
from pathlib import Path

import blake3
import pytest

PROJECT_ROOT = Path(__file__).parent.parent.parent
ASSETS_DIR = PROJECT_ROOT / "assets"


def host_arch() -> str:
    return "arm64" if os.uname().machine == "arm64" else "x86_64"


def asset_source_dir() -> Path:
    override = os.environ.get("CAPSEM_E2E_PROFILE_ASSET_SOURCE_DIR")
    if override:
        return Path(override)
    return ASSETS_DIR / host_arch()


def find_asset(source_dir: Path, logical_name: str) -> Path:
    exact = source_dir / logical_name
    if exact.exists():
        return exact
    patterns = {
        "vmlinuz": "vmlinuz-*",
        "initrd.img": "initrd-*.img",
        "rootfs.squashfs": "rootfs-*.squashfs",
    }
    matches = sorted(source_dir.glob(patterns[logical_name]))
    if matches:
        return matches[0]
    pytest.skip(f"missing {logical_name} under {source_dir}")


def blake3_hex(path: Path) -> str:
    hasher = blake3.blake3()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def toml_string(value: str) -> str:
    return json.dumps(value)


def write_profile_home(capsem_home: Path, asset_cache: Path, assets: dict[str, Path]):
    profile_dir = capsem_home / "profiles" / "corp"
    profile_dir.mkdir(parents=True)
    asset_cache.mkdir(parents=True)

    declarations = {
        logical_name: {
            "url": path.resolve().as_uri(),
            "hash": f"blake3:{blake3_hex(path)}",
            "size": path.stat().st_size,
            "content_type": (
                "application/vnd.squashfs"
                if logical_name == "rootfs.squashfs"
                else "application/octet-stream"
            ),
        }
        for logical_name, path in assets.items()
    }

    profile_content = f"""
version = 1
id = "profile-asset-boot"
name = "Profile Asset Boot"
description = "E2E profile proving profile-owned VM assets boot."
best_for = "Fresh profile asset download boot probes."
profile_type = "coding"

[vm]
memory_mib = 4096
cpus = 4

[vm.assets.{host_arch()}.kernel]
url = {toml_string(declarations["vmlinuz"]["url"])}
hash = {toml_string(declarations["vmlinuz"]["hash"])}
signature_url = {toml_string(declarations["vmlinuz"]["url"] + ".minisig")}
size = {declarations["vmlinuz"]["size"]}
content_type = {toml_string(declarations["vmlinuz"]["content_type"])}

[vm.assets.{host_arch()}.initrd]
url = {toml_string(declarations["initrd.img"]["url"])}
hash = {toml_string(declarations["initrd.img"]["hash"])}
signature_url = {toml_string(declarations["initrd.img"]["url"] + ".minisig")}
size = {declarations["initrd.img"]["size"]}
content_type = {toml_string(declarations["initrd.img"]["content_type"])}

[vm.assets.{host_arch()}.rootfs]
url = {toml_string(declarations["rootfs.squashfs"]["url"])}
hash = {toml_string(declarations["rootfs.squashfs"]["hash"])}
signature_url = {toml_string(declarations["rootfs.squashfs"]["url"] + ".minisig")}
size = {declarations["rootfs.squashfs"]["size"]}
content_type = {toml_string(declarations["rootfs.squashfs"]["content_type"])}
""".lstrip()
    profile_path = profile_dir / "profile-asset-boot.toml"
    profile_path.write_text(profile_content, encoding="utf-8")

    revision = "2026.0519.e2e"
    payload_hash = f"blake3:{blake3.blake3(profile_content.encode()).hexdigest()}"
    revision_dir = profile_dir / ".catalog" / "profiles" / "profile-asset-boot"
    (revision_dir / revision).mkdir(parents=True)
    (revision_dir / revision / "profile.json").write_text("{}", encoding="utf-8")
    (revision_dir / "current.json").write_text(
        json.dumps(
            {
                "profile_id": "profile-asset-boot",
                "revision": revision,
                "payload_hash": payload_hash,
            }
        ),
        encoding="utf-8",
    )

    (capsem_home / "service.toml").write_text(
        f"""
version = 1

[profiles]
corp_dirs = [{toml_string(str(profile_dir))}]
user_dirs = []
default_profile = "profile-asset-boot"

[assets]
assets_dir = {toml_string(str(asset_cache))}
""".lstrip(),
        encoding="utf-8",
    )
    return declarations
