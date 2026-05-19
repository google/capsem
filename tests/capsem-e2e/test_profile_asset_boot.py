"""Profile V2 asset download -> boot proof through real CLI + service.

This is intentionally an e2e/serial probe: it starts the real service against
an empty asset cache, points a Profile V2 config at real file-backed VM assets,
asks `capsem update --assets` to reconcile them, then boots and execs in a VM.
"""

from __future__ import annotations

import json
import os
import uuid
from pathlib import Path

import blake3
import pytest

pytestmark = [pytest.mark.e2e, pytest.mark.serial]

PROJECT_ROOT = Path(__file__).parent.parent.parent
ASSETS_DIR = PROJECT_ROOT / "assets"


def _host_arch() -> str:
    return "arm64" if os.uname().machine == "arm64" else "x86_64"


def _asset_source_dir() -> Path:
    override = os.environ.get("CAPSEM_E2E_PROFILE_ASSET_SOURCE_DIR")
    if override:
        return Path(override)
    return ASSETS_DIR / _host_arch()


def _find_asset(source_dir: Path, logical_name: str) -> Path:
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


def _blake3(path: Path) -> str:
    hasher = blake3.blake3()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def _toml_string(value: str) -> str:
    return json.dumps(value)


def _write_profile_home(capsem_home: Path, asset_cache: Path, assets: dict[str, Path]):
    profile_dir = capsem_home / "profiles" / "corp"
    profile_dir.mkdir(parents=True)
    asset_cache.mkdir(parents=True)

    declarations = {
        logical_name: {
            "url": path.resolve().as_uri(),
            "hash": f"blake3:{_blake3(path)}",
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

[vm.assets.{_host_arch()}.kernel]
url = {_toml_string(declarations["vmlinuz"]["url"])}
hash = {_toml_string(declarations["vmlinuz"]["hash"])}
signature_url = {_toml_string(declarations["vmlinuz"]["url"] + ".minisig")}
size = {declarations["vmlinuz"]["size"]}
content_type = {_toml_string(declarations["vmlinuz"]["content_type"])}

[vm.assets.{_host_arch()}.initrd]
url = {_toml_string(declarations["initrd.img"]["url"])}
hash = {_toml_string(declarations["initrd.img"]["hash"])}
signature_url = {_toml_string(declarations["initrd.img"]["url"] + ".minisig")}
size = {declarations["initrd.img"]["size"]}
content_type = {_toml_string(declarations["initrd.img"]["content_type"])}

[vm.assets.{_host_arch()}.rootfs]
url = {_toml_string(declarations["rootfs.squashfs"]["url"])}
hash = {_toml_string(declarations["rootfs.squashfs"]["hash"])}
signature_url = {_toml_string(declarations["rootfs.squashfs"]["url"] + ".minisig")}
size = {declarations["rootfs.squashfs"]["size"]}
content_type = {_toml_string(declarations["rootfs.squashfs"]["content_type"])}
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
corp_dirs = [{_toml_string(str(profile_dir))}]
user_dirs = []
default_profile = "profile-asset-boot"

[assets]
assets_dir = {_toml_string(str(asset_cache))}
""".lstrip(),
        encoding="utf-8",
    )
    return declarations


def test_profile_asset_download_boots_and_execs(tmp_path, real_service_factory):
    source_dir = _asset_source_dir()
    if not source_dir.exists():
        pytest.skip(f"asset source dir missing: {source_dir}")

    assets = {
        "vmlinuz": _find_asset(source_dir, "vmlinuz"),
        "initrd.img": _find_asset(source_dir, "initrd.img"),
        "rootfs.squashfs": _find_asset(source_dir, "rootfs.squashfs"),
    }
    capsem_home = tmp_path / "capsem-home"
    asset_cache = tmp_path / "downloaded-assets"

    declarations = _write_profile_home(capsem_home, asset_cache, assets)
    svc = real_service_factory(capsem_home=capsem_home, assets_dir=asset_cache)
    try:
        svc.start()

        update = svc.cli_ok("update", "--assets", timeout=240)
        assert "Profile VM assets reconciled" in update.stdout or "already ready" in update.stdout

        health = svc.api_json("GET", "/list")["asset_health"]
        assert health["ready"] is True
        assert health["state"] == "ready"
        assert health["profile_id"] == "profile-asset-boot"
        assert len(health["profile_assets"]) == 3

        for logical_name, declaration in declarations.items():
            installed = asset_cache / _host_arch()
            expected_name = {
                "vmlinuz": f"vmlinuz-{declaration['hash'][7:23]}",
                "initrd.img": f"initrd-{declaration['hash'][7:23]}.img",
                "rootfs.squashfs": f"rootfs-{declaration['hash'][7:23]}.squashfs",
            }[logical_name]
            assert (installed / expected_name).exists()
            assert any(
                asset["logical_name"] == logical_name
                and asset["hash"] == declaration["hash"]
                for asset in health["profile_assets"]
            )

        name = f"profileboot-{uuid.uuid4().hex[:8]}"
        try:
            svc.cli_ok("create", name, timeout=180)
            assert svc.wait_exec_ready(name, timeout=120)
            exec_result = svc.cli_ok("exec", name, "echo profile-asset-boot-ok", timeout=60)
            assert "profile-asset-boot-ok" in exec_result.stdout
            info = json.loads(svc.cli_ok("info", "--json", name, timeout=60).stdout)
            assert info["profile_id"] == "profile-asset-boot"
            assert info["profile_revision"] == "2026.0519.e2e"
        finally:
            svc.cli("delete", name, timeout=60)
    finally:
        svc.stop()
