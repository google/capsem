"""Profile V2 asset download -> boot proof through real CLI + service.

This is intentionally an e2e/serial probe: it starts the real service against
an empty asset cache, points a Profile V2 config at real file-backed VM assets,
asks `capsem update --assets` to reconcile them, then boots and execs in a VM.
"""

from __future__ import annotations

import json
import subprocess
import uuid

import pytest

from capsem.builder.image_verify import ImageInventory
from helpers.profile_asset_fixture import (
    asset_source_dir,
    find_asset,
    host_arch,
    write_profile_home,
)

pytestmark = [pytest.mark.e2e, pytest.mark.serial]

def test_profile_asset_download_boots_and_execs(tmp_path, real_service_factory):
    source_dir = asset_source_dir()
    if not source_dir.exists():
        pytest.skip(f"asset source dir missing: {source_dir}")

    assets = {
        "vmlinuz": find_asset(source_dir, "vmlinuz"),
        "initrd.img": find_asset(source_dir, "initrd.img"),
        "rootfs.squashfs": find_asset(source_dir, "rootfs.squashfs"),
    }
    capsem_home = tmp_path / "capsem-home"
    asset_cache = tmp_path / "downloaded-assets"

    declarations = write_profile_home(capsem_home, asset_cache, assets)
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
            installed = asset_cache / host_arch()
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


def test_profile_asset_doctor_bundle_is_admin_verified(
    tmp_path, real_service_factory
):
    source_dir = asset_source_dir()
    if not source_dir.exists():
        pytest.skip(f"asset source dir missing: {source_dir}")
    inventory_path = source_dir / "image-inventory.json"
    if not inventory_path.exists():
        pytest.skip(f"image inventory missing: {inventory_path}")

    inventory = ImageInventory.model_validate_json(
        inventory_path.read_text(encoding="utf-8")
    )
    assets = {
        "vmlinuz": find_asset(source_dir, "vmlinuz"),
        "initrd.img": find_asset(source_dir, "initrd.img"),
        "rootfs.squashfs": find_asset(source_dir, "rootfs.squashfs"),
    }
    capsem_home = tmp_path / "capsem-home"
    asset_cache = tmp_path / "downloaded-assets"

    write_profile_home(
        capsem_home,
        asset_cache,
        assets,
        image_inventory=inventory,
    )
    profile_json = (
        capsem_home
        / "profiles"
        / "corp"
        / ".catalog"
        / "profiles"
        / "profile-asset-boot"
        / "2026.0519.e2e"
        / "profile.json"
    )
    svc = real_service_factory(capsem_home=capsem_home, assets_dir=asset_cache)
    try:
        svc.start()
        update = svc.cli_ok("update", "--assets", timeout=240)
        assert (
            "Profile VM assets reconciled" in update.stdout
            or "already ready" in update.stdout
        )

        doctor = svc.cli_ok("doctor", "--fast", "--bundle", timeout=420)
        assert "RESULT: PASS" in doctor.stdout
        bundle_path = svc.tmp_dir / "doctor-latest.tar"
        assert bundle_path.is_file()

        result = subprocess.run(
            [
                "uv",
                "run",
                "capsem-admin",
                "image",
                "verify",
                str(profile_json),
                "--assets-dir",
                str(source_dir.parent),
                "--arch",
                host_arch(),
                "--inventory",
                str(inventory_path),
                "--doctor-bundle",
                str(bundle_path),
                "--json",
            ],
            capture_output=True,
            text=True,
            timeout=120,
            env=svc.env,
        )
        assert result.returncode == 0, (
            f"capsem-admin image verify failed\n"
            f"stdout: {result.stdout}\nstderr: {result.stderr}"
        )
        report = json.loads(result.stdout)
        assert report["ok"] is True
        assert report["profile_id"] == "profile-asset-boot"
        assert report["probes"][0]["kind"] == "capsem_doctor_bundle"
        assert report["probes"][0]["ok"] is True
    finally:
        svc.stop()
