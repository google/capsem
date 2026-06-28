"""Ironbank profile asset readiness contract.

The profile card is only allowed to reflect route-owned truth. This test starts
the real service against a generated profile manifest and proves the route
ledger that the UI consumes: missing assets block readiness, ensure downloads
through the normal downloader, and ready assets are hash-named with exact
kernel/initrd/rootfs facts.
"""

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


def _materialize_profile(
    *,
    profile_id: str,
    output_root: Path,
    source_assets: Path,
    manifest: Path,
    arch: str,
    clean: bool,
) -> None:
    command = [
        str(_ensure_capsem_admin()),
        "profile",
        "materialize",
        "--profile",
        str(PROJECT_ROOT / "config" / "profiles" / profile_id / "profile.toml"),
        "--config-root",
        str(PROJECT_ROOT / "config"),
        "--manifest",
        manifest.resolve().as_uri(),
        "--assets-dir",
        str(source_assets),
        "--output-root",
        str(output_root),
        "--arch",
        arch,
        "--json",
    ]
    if clean:
        command.append("--clean")
    result = subprocess.run(
        command,
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        timeout=60,
    )
    assert result.returncode == 0, (
        f"profile materialize failed for {profile_id}\n"
        f"stdout={result.stdout}\nstderr={result.stderr}"
    )


def _seed_profiles(tmp_path: Path) -> tuple[Path, Path, dict[str, bytes], Path]:
    arch = _arch()
    source_assets = tmp_path / "source-assets"
    files = {
        "vmlinuz": b"ironbank-profile-kernel",
        "initrd.img": b"ironbank-profile-initrd",
        "rootfs.erofs": b"ironbank-profile-rootfs",
    }
    manifest = _write_manifest(source_assets, arch, files)
    output_root = tmp_path / "runtime-config"
    _materialize_profile(
        profile_id="code",
        output_root=output_root,
        source_assets=source_assets,
        manifest=manifest,
        arch=arch,
        clean=True,
    )
    _materialize_profile(
        profile_id="co-work",
        output_root=output_root,
        source_assets=source_assets,
        manifest=manifest,
        arch=arch,
        clean=False,
    )
    return output_root / "profiles", source_assets, files, manifest


def _expected_assets(files: dict[str, bytes], installed_assets: Path, arch: str) -> dict[str, dict]:
    by_kind = {
        "kernel": ("vmlinuz", files["vmlinuz"]),
        "initrd": ("initrd.img", files["initrd.img"]),
        "rootfs": ("rootfs.erofs", files["rootfs.erofs"]),
    }
    expected = {}
    for kind, (logical_name, data) in by_kind.items():
        digest = _blake3(data)
        name = _hash_filename(logical_name, digest)
        expected[kind] = {
            "name": name,
            "expected_hash": f"blake3:{digest}",
            "expected_size": len(data),
            "path": installed_assets / arch / name,
            "data": data,
        }
    return expected


def test_profile_cards_can_be_built_from_asset_readiness_routes(tmp_path: Path) -> None:
    profiles, _source_assets, files, manifest = _seed_profiles(tmp_path)
    installed_assets = tmp_path / "installed-assets"
    service = ServiceInstance(assets_dir=installed_assets)
    service.profiles_dir = profiles
    service.start()
    try:
        client = service.client()

        listed = client.get("/profiles/list")
        listed_by_id = {profile["id"]: profile for profile in listed["profiles"]}
        assert set(listed_by_id) == {"code", "co-work"}
        assert listed_by_id["code"]["name"] == "Code"
        assert listed_by_id["code"]["description"] == "Optimized for coding and long-running agents."
        assert listed_by_id["co-work"]["name"] == "Co-work"
        assert listed_by_id["co-work"]["description"] == "Shared profile for collaborative agent sessions."
        for profile in listed_by_id.values():
            assert profile["icon_svg"].startswith("<svg")
            assert profile["availability"] == {"web": True, "shell": True, "mobile": True}
            assert "policy" not in profile
            assert "enabled_by" not in profile

        initial_status = client.get("/profiles/status")
        assert initial_status["profile_count"] == 2
        assert initial_status["ready_count"] == 0
        initial_by_id = {profile["id"]: profile for profile in initial_status["profiles"]}
        for profile_id, profile_status in initial_by_id.items():
            assert profile_status["ready"] is False
            assert profile_status["asset_count"] == 3
            assert {asset["kind"] for asset in profile_status["missing_assets"]} == {
                "kernel",
                "initrd",
                "rootfs",
            }, profile_id
            assert {asset["kind"] for asset in profile_status["invalid_assets"]} == {
                "kernel",
                "initrd",
                "rootfs",
            }, profile_id
            assert all(asset["present"] is False for asset in profile_status["invalid_assets"])
            assert all(asset["valid"] is False for asset in profile_status["invalid_assets"])

            route_status = client.get(f"/profiles/{profile_id}/assets/status")
            assert route_status["profile_id"] == profile_id
            assert route_status["ready"] is False
            assert route_status["missing_assets"] == profile_status["missing_assets"]
            assert route_status["invalid_assets"] == profile_status["invalid_assets"]
            assert route_status["manifest"]["origin"] == "missing"
            assert route_status["manifest"]["validation_status"] == "missing"
            assert {asset["kind"] for asset in route_status["assets"]} == {
                "kernel",
                "initrd",
                "rootfs",
            }
            assert {asset["status"] for asset in route_status["assets"]} == {"missing"}

        for index, profile_id in enumerate(("code", "co-work")):
            ensured = client.post(f"/profiles/{profile_id}/assets/ensure", {}, timeout=30)
            assert ensured["profile_id"] == profile_id
            assert ensured["ensured"] is True
            assert ensured["downloaded"] == (3 if index == 0 else 0)
            assert ensured["ready"] is True
            assert ensured["missing_assets"] == []
            assert ensured["invalid_assets"] == []

            expected = _expected_assets(files, installed_assets, _arch())
            actual_by_kind = {asset["kind"]: asset for asset in ensured["assets"]}
            assert set(actual_by_kind) == {"kernel", "initrd", "rootfs"}
            for kind, asset in actual_by_kind.items():
                want = expected[kind]
                assert asset["status"] == "present"
                assert asset["name"] == want["name"]
                assert asset["expected_hash"] == want["expected_hash"]
                assert asset["expected_size"] == want["expected_size"]
                assert asset["actual_size"] == want["expected_size"]
                assert Path(asset["path"]) == want["path"]
                assert Path(asset["path"]).read_bytes() == want["data"]

        shutil.copy2(manifest, installed_assets / "manifest.json")
        final_status = client.get("/profiles/status")
        assert final_status["ready_count"] == 2
        assert final_status["asset_manifest"]["format"] == 2
        assert final_status["asset_manifest"]["refresh_policy"] == "24h"
        assert final_status["asset_manifest"]["assets_current"] == "2099.0101.1"
        assert final_status["asset_manifest"]["blake3"] == _blake3(manifest.read_bytes())
        for profile in final_status["profiles"]:
            assert profile["ready"] is True
            assert profile["missing_assets"] == []
            assert profile["invalid_assets"] == []
    finally:
        service.stop()
