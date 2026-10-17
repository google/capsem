"""Ironbank runtime asset readiness contract.

The asset card and the launch button may only reflect route-owned truth. This
test starts the real service against an installed format-2 manifest and its
hash-named assets and proves the route ledger the UI consumes: present assets
are ready with exact kernel/initrd/rootfs facts, a missing asset makes the set
not ready and refuses VM creation by name, and restoring it makes the set
ready again.
"""

from __future__ import annotations

import json
import platform
import subprocess
from pathlib import Path

from helpers.service import ServiceInstance

ASSET_VERSION = "2099.0101.1"
FILES = {
    "vmlinuz": b"ironbank-runtime-kernel",
    "initrd.img": b"ironbank-runtime-initrd",
    "rootfs.erofs": b"ironbank-runtime-rootfs",
}
KINDS = {"vmlinuz": "kernel", "initrd.img": "initrd", "rootfs.erofs": "rootfs"}


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


def _install(installed: Path, arch: str) -> Path:
    (installed / arch).mkdir(parents=True)
    for name, data in FILES.items():
        (installed / arch / _hash_filename(name, _blake3(data))).write_bytes(data)
    manifest = {
        "format": 2,
        "refresh_policy": "24h",
        "assets": {
            "current": ASSET_VERSION,
            "releases": {
                ASSET_VERSION: {
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
                    "min_assets": ASSET_VERSION,
                }
            },
        },
    }
    path = installed / "manifest.json"
    path.write_text(json.dumps(manifest), encoding="utf-8")
    return path


def _assert_exact_assets(status: dict, installed: Path, arch: str, *, missing: str | None) -> None:
    by_name = {asset["name"]: asset for asset in status["assets"]}
    assert set(by_name) == set(FILES), status
    for name, data in FILES.items():
        asset = by_name[name]
        digest = _blake3(data)
        path = installed / arch / _hash_filename(name, digest)
        assert asset["kind"] == KINDS[name]
        assert asset["expected_hash"].removeprefix("blake3:") == digest
        assert asset["expected_size"] == len(data)
        if name == missing:
            # A file found nowhere resolves to the flat layout's path.
            assert asset["path"] == str(installed / _hash_filename(name, digest))
            assert asset["status"] == "missing"
            assert "actual_size" not in asset
        else:
            assert asset["path"] == str(path)
            assert asset["status"] == "present"
            assert asset["actual_size"] == len(data)
            assert path.read_bytes() == data


def test_asset_cards_and_launch_follow_the_asset_readiness_route(tmp_path: Path) -> None:
    arch = _arch()
    installed = tmp_path / "installed-assets"
    manifest = _install(installed, arch)
    rootfs = installed / arch / _hash_filename("rootfs.erofs", _blake3(FILES["rootfs.erofs"]))

    service = ServiceInstance(assets_dir=installed)
    service.start()
    try:
        client = service.client()

        ready = client.get("/assets/status")
        assert ready["ready"] is True, ready
        assert ready["downloading"] is False
        assert ready["errors"] == []
        assert ready["current_arch"] == arch
        assert ready["asset_version"] == ASSET_VERSION
        _assert_exact_assets(ready, installed, arch, missing=None)
        assert ready["manifest"]["validation_status"] == "valid"
        assert ready["manifest"]["format"] == 2
        assert ready["manifest"]["refresh_policy"] == "24h"
        assert ready["manifest"]["assets_current"] == ASSET_VERSION
        assert ready["manifest"]["blake3"] == _blake3(manifest.read_bytes())

        kept = rootfs.read_bytes()
        rootfs.unlink()
        missing = client.get("/assets/status")
        assert missing["ready"] is False
        _assert_exact_assets(missing, installed, arch, missing="rootfs.erofs")
        assert missing["errors"] == [
            f"rootfs.erofs is missing at {installed / rootfs.name}"
        ], missing

        status, refused = client.call_json(
            "POST", "/vms/create", {"name": "asset-gated", "ram_mb": 2048, "cpus": 2}, timeout=30
        )
        assert status >= 400, (status, refused)
        assert "VM assets are not ready" in refused["error"], refused
        assert "rootfs.erofs" in refused["error"], refused
        assert all(row["name"] != "asset-gated" for row in client.get("/vms/list")["sandboxes"])

        rootfs.write_bytes(kept)
        restored = client.get("/assets/status")
        assert restored["ready"] is True, restored
        _assert_exact_assets(restored, installed, arch, missing=None)
    finally:
        service.stop()
