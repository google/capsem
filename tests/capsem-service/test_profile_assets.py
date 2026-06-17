"""Profile asset readiness and hydration route contract."""

from __future__ import annotations

import json
import platform
import shutil
import subprocess
from pathlib import Path

from helpers.service import PROJECT_ROOT, ServiceInstance


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


def _write_manifest(source_assets: Path, arch: str, files: dict[str, bytes]) -> Path:
    (source_assets / arch).mkdir(parents=True)
    for name, data in files.items():
        (source_assets / arch / name).write_bytes(data)
    manifest = {
        "format": 2,
        "refresh_policy": "24h",
        "assets": {
            "current": "2099.0101.1",
            "releases": {
                "2099.0101.1": {
                    "date": "2099-01-01",
                    "deprecated": False,
                    "min_binary": "1.0.0",
                    "arches": {
                        arch: {
                            name: {"hash": _blake3(data), "size": len(data)}
                            for name, data in files.items()
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
    manifest_path = source_assets / "manifest.json"
    manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
    return manifest_path


def _ensure_capsem_admin() -> Path:
    binary = PROJECT_ROOT / "target" / "debug" / "capsem-admin"
    if not binary.exists():
        subprocess.run(
            ["cargo", "build", "-p", "capsem-admin"],
            cwd=PROJECT_ROOT,
            check=True,
            timeout=120,
        )
    return binary


def _materialize_code_profile(tmp_path: Path, source_assets: Path, manifest: Path, arch: str) -> Path:
    output_root = tmp_path / "runtime-config"
    result = subprocess.run(
        [
            str(_ensure_capsem_admin()),
            "profile",
            "materialize",
            "--profile",
            str(PROJECT_ROOT / "config" / "profiles" / "code" / "profile.toml"),
            "--config-root",
            str(PROJECT_ROOT / "config"),
            "--manifest",
            str(manifest),
            "--assets-dir",
            str(source_assets),
            "--output-root",
            str(output_root),
            "--arch",
            arch,
            "--clean",
            "--json",
        ],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        timeout=60,
    )
    assert result.returncode == 0, (
        f"profile materialize failed\nstdout={result.stdout}\nstderr={result.stderr}"
    )
    profiles = output_root / "profiles"
    # Keep the fixture focused on one materialized profile; copied source
    # profiles are not the subject of this route contract.
    for child in profiles.iterdir():
        if child.name != "code":
            if child.is_dir():
                shutil.rmtree(child)
            else:
                child.unlink()
    return profiles


def _seed_profile_fixture(tmp_path: Path) -> tuple[Path, Path, dict[str, bytes], Path]:
    arch = _arch()
    source_assets = tmp_path / "source-assets"
    files = {
        "vmlinuz": b"profile-assets-kernel",
        "initrd.img": b"profile-assets-initrd",
        "rootfs.erofs": b"profile-assets-rootfs",
    }
    manifest = _write_manifest(source_assets, arch, files)
    profiles = _materialize_code_profile(tmp_path, source_assets, manifest, arch)
    return profiles, source_assets, files, manifest


def test_profile_asset_routes_gate_start_until_hash_named_assets_are_hydrated(
    tmp_path: Path,
) -> None:
    profiles, _source_assets, files, _manifest = _seed_profile_fixture(tmp_path)
    installed_assets = tmp_path / "installed-assets"
    service = ServiceInstance(assets_dir=installed_assets)
    service.profiles_dir = profiles
    service.start()
    try:
        client = service.client()

        status = client.get("/profiles/status")
        assert status["profile_count"] == 1
        assert status["ready_count"] == 0
        profile = status["profiles"][0]
        assert profile["id"] == "code"
        assert profile["ready"] is False
        assert {asset["kind"] for asset in profile["missing_assets"]} == {
            "kernel",
            "initrd",
            "rootfs",
        }

        assets = client.get("/profiles/code/assets/status")
        assert assets["profile_id"] == "code"
        assert assets["ready"] is False
        assert assets["manifest"]["origin"] == "missing"
        assert {asset["status"] for asset in assets["assets"]} == {"missing"}

        ensured = client.post("/profiles/code/assets/ensure", {}, timeout=30)
        assert ensured["ensured"] is True
        assert ensured["downloaded"] == 3
        assert ensured["ready"] is True
        assert ensured["missing_assets"] == []
        assert ensured["invalid_assets"] == []
        assert {asset["status"] for asset in ensured["assets"]} == {"present"}

        arch = _arch()
        data_by_kind = {
            "kernel": files["vmlinuz"],
            "initrd": files["initrd.img"],
            "rootfs": files["rootfs.erofs"],
        }
        for asset in ensured["assets"]:
            data = data_by_kind[asset["kind"]]
            digest = _blake3(data)
            logical_name = {
                "kernel": "vmlinuz",
                "initrd": "initrd.img",
                "rootfs": "rootfs.erofs",
            }[asset["kind"]]
            expected_name = _hash_filename(logical_name, digest)
            assert asset["name"] == expected_name
            assert asset["expected_hash"] == f"blake3:{digest}"
            assert asset["expected_size"] == len(data)
            assert asset["actual_size"] == len(data)
            assert Path(asset["path"]) == installed_assets / arch / expected_name
            assert Path(asset["path"]).read_bytes() == data

        refreshed = client.get("/profiles/status")
        assert refreshed["ready_count"] == 1
        assert refreshed["profiles"][0]["ready"] is True
        assert refreshed["profiles"][0]["missing_assets"] == []
    finally:
        service.stop()


def test_profile_asset_routes_report_manifest_origin_hash_and_validity(tmp_path: Path) -> None:
    profiles, source_assets, files, manifest = _seed_profile_fixture(tmp_path)
    arch = _arch()
    installed_assets = tmp_path / "installed-assets"
    (installed_assets / arch).mkdir(parents=True)
    shutil.copy2(manifest, installed_assets / "manifest.json")
    (installed_assets / "manifest-origin.json").write_text(
        json.dumps(
            {
                "schema": "capsem.manifest_origin.v1",
                "origin": "package",
                "source": manifest.as_uri(),
                "packaged_at": "2026-06-16T00:00:00Z",
            },
            sort_keys=True,
        )
        + "\n",
        encoding="utf-8",
    )
    for logical_name, data in files.items():
        digest = _blake3(data)
        shutil.copy2(
            source_assets / arch / logical_name,
            installed_assets / arch / _hash_filename(logical_name, digest),
        )

    service = ServiceInstance(assets_dir=installed_assets)
    service.profiles_dir = profiles
    service.start()
    try:
        client = service.client()
        assets = client.get("/profiles/code/assets/status")
        assert assets["ready"] is True
        manifest_status = assets["manifest"]
        assert manifest_status["origin"] == "package"
        assert manifest_status["origin_source"] == manifest.as_uri()
        assert manifest_status["packaged_at"] == "2026-06-16T00:00:00Z"
        assert manifest_status["validation_status"] == "valid"
        assert manifest_status["format"] == 2
        assert manifest_status["refresh_policy"] == "24h"
        assert manifest_status["assets_current"] == "2099.0101.1"
        assert manifest_status["blake3"] == _blake3(manifest.read_bytes())

        profiles_status = client.get("/profiles/status")
        assert profiles_status["asset_manifest"] == manifest_status
        assert profiles_status["profiles"][0]["ready"] is True
    finally:
        service.stop()
