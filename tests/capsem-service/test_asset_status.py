"""Runtime asset readiness route contract: `/assets/status` and `/assets/ensure`.

There is one runtime asset set, the installed manifest's release for this
binary on this host's architecture.
"""

from __future__ import annotations

import json
import platform
import subprocess
from pathlib import Path

from helpers.service import ServiceInstance, wait_assets_settled

BOOT_ASSETS = {"vmlinuz": "kernel", "initrd.img": "initrd", "rootfs.erofs": "rootfs"}
FILES = {
    "vmlinuz": b"runtime-assets-kernel",
    "initrd.img": b"runtime-assets-initrd",
    "rootfs.erofs": b"runtime-assets-rootfs",
}


def _arch() -> str:
    machine = platform.machine().lower()
    return "arm64" if machine in ("arm64", "aarch64") else "x86_64"


def _blake3(data: bytes) -> str:
    try:
        import blake3 as b3  # type: ignore

        return b3.blake3(data).hexdigest()
    except ImportError:
        result = subprocess.run(
            ["b3sum", "--no-names"],
            input=data,
            capture_output=True,
            check=True,
        )
        return result.stdout.decode().strip().split()[0]


def _hash_filename(logical_name: str, digest: str) -> str:
    prefix = digest[:16]
    if "." in logical_name:
        stem, ext = logical_name.split(".", 1)
        return f"{stem}-{prefix}.{ext}"
    return f"{logical_name}-{prefix}"


def _install_assets(installed: Path, arch: str) -> Path:
    """Install a format-2 manifest and its hash-named assets."""
    (installed / arch).mkdir(parents=True)
    for name, data in FILES.items():
        (installed / arch / _hash_filename(name, _blake3(data))).write_bytes(data)
    manifest = {
        "format": 2,
        "refresh_policy": "24h",
        "assets": {
            "current": "2099.0101.1",
            "releases": {
                "2099.0101.1": {
                    "date": "2099-01-01",
                    "deprecated": False,
                    "min_binary": "0.0.0",
                    "arches": {
                        arch: {
                            name: {"hash": _blake3(data), "size": len(data)}
                            for name, data in FILES.items()
                        }
                    },
                }
            },
        },
        "binaries": {
            "current": "1.0.0",
            "releases": {
                "1.0.0": {
                    "date": "2099-01-01",
                    "deprecated": False,
                    "min_assets": "2099.0101.1",
                }
            },
        },
    }
    manifest_path = installed / "manifest.json"
    manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
    return manifest_path


def test_asset_status_without_a_manifest_is_not_ready_and_ensure_settles(tmp_path: Path) -> None:
    service = ServiceInstance(assets_dir=tmp_path / "installed-assets")
    service.start()
    try:
        client = service.client()
        status = client.get("/assets/status")
        assert status["ready"] is False
        assert status["current_arch"] == _arch()
        assert status["assets"] == []
        assert status["errors"], status
        assert status["manifest"]["origin"] == "missing"
        assert status["manifest"]["validation_status"] == "missing"
        assert "profile_id" not in status

        ensured = client.post("/assets/ensure", {}, timeout=30)
        assert isinstance(ensured["started"], bool), ensured
        settled = wait_assets_settled(client)
        assert settled["ready"] is False
        assert settled["downloading"] is False
        assert "started" not in settled
    finally:
        service.stop()


def test_asset_status_reports_manifest_metadata_hash_validity_and_assets(tmp_path: Path) -> None:
    arch = _arch()
    installed = tmp_path / "installed-assets"
    manifest = _install_assets(installed, arch)
    (installed / "manifest-metadata.json").write_text(
        json.dumps(
            {
                "schema": "capsem.manifest_metadata.v1",
                "origin": "package",
                "manifest_url": manifest.as_uri(),
                "packaged_at": "2026-06-16T00:00:00Z",
            },
            sort_keys=True,
        )
        + "\n",
        encoding="utf-8",
    )

    service = ServiceInstance(assets_dir=installed)
    service.start()
    try:
        client = service.client()
        status = client.get("/assets/status")
        assert status["ready"] is True, status
        assert status["downloading"] is False
        assert status["errors"] == []
        assert status["current_arch"] == arch
        assert status["asset_version"] == "2099.0101.1"

        by_name = {asset["name"]: asset for asset in status["assets"]}
        assert set(by_name) == set(BOOT_ASSETS)
        for name, data in FILES.items():
            asset = by_name[name]
            digest = _blake3(data)
            assert asset["kind"] == BOOT_ASSETS[name]
            assert asset["status"] == "present"
            assert asset["expected_hash"].removeprefix("blake3:") == digest
            assert asset["expected_size"] == len(data)
            assert asset["actual_size"] == len(data)
            assert Path(asset["path"]) == installed / arch / _hash_filename(name, digest)

        manifest_status = status["manifest"]
        assert manifest_status["origin"] == "package"
        assert manifest_status["origin_source"] == manifest.as_uri()
        assert manifest_status["packaged_at"] == "2026-06-16T00:00:00Z"
        assert manifest_status["validation_status"] == "valid"
        assert manifest_status["format"] == 2
        assert manifest_status["refresh_policy"] == "24h"
        assert manifest_status["assets_current"] == "2099.0101.1"
        assert manifest_status["blake3"] == _blake3(manifest.read_bytes())

        assert client.get("/system/status")["assets"] == status
    finally:
        service.stop()

