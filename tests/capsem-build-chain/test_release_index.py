"""Release-channel index generator contract tests."""

from __future__ import annotations

import hashlib
import json
import subprocess
from pathlib import Path

from blake3 import blake3


PROJECT_ROOT = Path(__file__).resolve().parents[2]


def _run_admin(*args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(
        ["cargo", "run", "-p", "capsem-admin", "--quiet", "--", *args],
        cwd=PROJECT_ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if check and result.returncode != 0:
        raise AssertionError(
            f"capsem-admin {' '.join(args)} failed\n"
            f"stdout:\n{result.stdout}\n"
            f"stderr:\n{result.stderr}"
        )
    return result


def _write_asset(path: Path, data: bytes) -> dict[str, object]:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
    return {"hash": blake3(data).hexdigest(), "size": len(data)}


def _write_release_manifest(root: Path) -> Path:
    assets = root / "assets"
    arm64 = assets / "arm64"
    files = {
        "vmlinuz": _write_asset(arm64 / "vmlinuz", b"kernel-arm64"),
        "initrd.img": _write_asset(arm64 / "initrd.img", b"initrd-arm64"),
        "rootfs.erofs": _write_asset(arm64 / "rootfs.erofs", b"rootfs-arm64"),
        "obom.cdx.json": _write_asset(
            arm64 / "obom.cdx.json",
            b'{"bomFormat":"CycloneDX","metadata":{"tools":[{"name":"cdxgen"}]}}',
        ),
    }
    pkg = b"pkg bytes"
    sbom = b'{"spdxVersion":"SPDX-2.3"}'
    manifest = {
        "format": 2,
        "refresh_policy": "24h",
        "assets": {
            "current": "2030.0101.1",
            "releases": {
                "2030.0101.1": {
                    "date": "2030-01-01",
                    "deprecated": False,
                    "min_binary": "1.4.0",
                    "arches": {"arm64": files},
                }
            },
        },
        "binaries": {
            "current": "1.4.1234567890",
            "releases": {
                "1.4.1234567890": {
                    "date": "2030-01-01",
                    "deprecated": False,
                    "min_assets": "2030.0101.1",
                    "files": [
                        {
                            "name": "Capsem-1.4.1234567890.pkg",
                            "size": len(pkg),
                            "sha256": hashlib.sha256(pkg).hexdigest(),
                        },
                        {
                            "name": "capsem-sbom.spdx.json",
                            "size": len(sbom),
                            "sha256": hashlib.sha256(sbom).hexdigest(),
                        },
                    ],
                }
            },
        },
    }
    manifest_path = assets / "manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return manifest_path


def _write_profile_catalog(root: Path, revision: str = "profiles-2030.0101.1") -> Path:
    profiles_dir = root / "config" / "profiles"
    profile_dir = profiles_dir / "code"
    profile_dir.mkdir(parents=True, exist_ok=True)
    (profile_dir / "profile.toml").write_text(
        f"""
id = "code"
name = "Code"
description = "Profile catalog fixture."
revision = "{revision}"
refresh_policy = "24h"

[assets]
format = "profile-assets.v1"
refresh_policy = "on_profile_refresh"

[assets.arch.arm64.kernel]
name = "vmlinuz"
url = "https://release.capsem.org/assets/releases/2030.0101.1/arm64-vmlinuz"

[assets.arch.arm64.initrd]
name = "initrd.img"
url = "https://release.capsem.org/assets/releases/2030.0101.1/arm64-initrd.img"

[assets.arch.arm64.rootfs]
name = "rootfs.erofs"
url = "https://release.capsem.org/assets/releases/2030.0101.1/arm64-rootfs.erofs"
""".strip()
        + "\n",
        encoding="utf-8",
    )
    return profiles_dir


def test_release_index_generator_builds_human_and_machine_outputs(tmp_path: Path) -> None:
    manifest_path = _write_release_manifest(tmp_path)
    profiles_dir = _write_profile_catalog(tmp_path)
    dist = tmp_path / "target" / "release-channel"

    result = _run_admin(
        "assets",
        "channel",
        "build",
        "--manifest",
        f"file://{manifest_path}",
        "--assets-dir",
        str(manifest_path.parent),
        "--profiles-dir",
        str(profiles_dir),
        "--channel",
        "stable",
        "--out-dir",
        str(dist),
        "--json",
    )

    report = json.loads(result.stdout)
    assert report["schema"] == "capsem.admin.assets_channel_build.v1"
    assert report["channel"] == "stable"
    assert report["manifest"] == str(dist / "assets" / "stable" / "manifest.json")
    assert report["copied_assets"] == 4

    index_html = (dist / "index.html").read_text(encoding="utf-8")
    assert "Capsem Asset Channel" in index_html
    assert "Current Asset Files" in index_html
    assert "Host SBOM Evidence" in index_html
    assert "VM OBOM Evidence" in index_html
    assert "Update Contract" in index_html
    assert "Profile Catalog" in index_html
    assert "profiles-2030.0101.1" in index_html
    assert "Realm Discipline" in index_html
    assert "/assets/releases/2030.0101.1/arm64-rootfs.erofs" in index_html
    assert "/assets/releases/2030.0101.1/arm64-obom.cdx.json" in index_html
    assert "capsem-sbom.spdx.json" in index_html
    assert "The fastest way to ship with AI securely." not in index_html

    headers = (dist / "_headers").read_text(encoding="utf-8")
    assert "/health.json\n  Cache-Control: no-cache, must-revalidate" in headers
    assert "/assets/*\n  Cache-Control: no-cache, must-revalidate" in headers
    assert "/profiles/*\n  Cache-Control: no-cache, must-revalidate" in headers

    health = json.loads((dist / "health.json").read_text(encoding="utf-8"))
    assert health["schema"] == "capsem.assets_channel.health.v1"
    assert health["urls"]["manifest"] == "/assets/stable/manifest.json"
    assert health["urls"]["asset_base"] == "/assets/releases"
    assert health["current"] == {
        "binary": "1.4.1234567890",
        "assets": "2030.0101.1",
    }
    assert health["updates"]["binary"]["latest"] == "1.4.1234567890"
    assert health["updates"]["assets"]["manifest"] == "/assets/stable/manifest.json"
    catalog_url = "/profiles/releases/profiles-2030.0101.1/catalog.json"
    assert health["profiles"]["revision"] == "profiles-2030.0101.1"
    assert health["profiles"]["source"] == catalog_url
    assert len(health["profiles"]["hash"]) == 64
    assert health["profiles"]["compatibility"] == {
        "binary": "1.4.1234567890",
        "assets": "2030.0101.1",
        "min_binary": "1.4.0",
        "min_assets": "2030.0101.1",
    }
    assert health["profiles"]["requires_newer"] == {
        "binary": False,
        "assets": False,
    }
    assert health["updates"]["profiles"]["latest"] == "profiles-2030.0101.1"
    assert health["updates"]["profiles"]["current"] == "profiles-2030.0101.1"
    assert health["updates"]["profiles"]["state"] == "current"
    assert health["updates"]["profiles"]["source"] == catalog_url
    assert health["updates"]["profiles"]["hash"] == health["profiles"]["hash"]
    assert health["updates"]["profiles"]["compatibility"] == health["profiles"]["compatibility"]
    assert health["updates"]["profiles"]["requires_newer"] == health["profiles"]["requires_newer"]
    catalog_path = dist / catalog_url.removeprefix("/")
    catalog_bytes = catalog_path.read_bytes()
    catalog_text = catalog_bytes.decode()
    assert health["profiles"]["hash"] == blake3(catalog_bytes).hexdigest()
    assert "file://" not in catalog_text
    assert str(tmp_path) not in catalog_text
    assert "https://release.capsem.org/assets/releases/2030.0101.1/arm64-rootfs.erofs" in (
        catalog_text
    )
    assert health["updates"]["images"]["latest"] is None
    assert health["evidence"]["vm_oboms"][0]["url"] == (
        "/assets/releases/2030.0101.1/arm64-obom.cdx.json"
    )
    assert health["evidence"]["host_sboms"][0]["name"] == "capsem-sbom.spdx.json"

    release_dir = dist / "assets" / "releases" / "2030.0101.1"
    assert (dist / "assets" / "stable" / "manifest.json").is_file()
    assert (release_dir / "arm64-vmlinuz").read_bytes() == b"kernel-arm64"
    assert (release_dir / "arm64-initrd.img").read_bytes() == b"initrd-arm64"
    assert (release_dir / "arm64-rootfs.erofs").read_bytes() == b"rootfs-arm64"
    assert (release_dir / "arm64-obom.cdx.json").is_file()

    _run_admin("assets", "channel", "check", "--channel", "stable", "--dist", str(dist))


def test_release_index_check_rejects_profile_catalog_index_drift(tmp_path: Path) -> None:
    manifest_path = _write_release_manifest(tmp_path)
    profiles_dir = _write_profile_catalog(tmp_path)
    dist = tmp_path / "target" / "release-channel"
    _run_admin(
        "assets",
        "channel",
        "build",
        "--manifest",
        f"file://{manifest_path}",
        "--assets-dir",
        str(manifest_path.parent),
        "--profiles-dir",
        str(profiles_dir),
        "--channel",
        "stable",
        "--out-dir",
        str(dist),
    )

    health_path = dist / "health.json"
    health = json.loads(health_path.read_text(encoding="utf-8"))
    health["updates"]["profiles"]["latest"] = "profiles-stale"
    health_path.write_text(json.dumps(health, indent=2) + "\n", encoding="utf-8")

    result = _run_admin(
        "assets",
        "channel",
        "check",
        "--channel",
        "stable",
        "--dist",
        str(dist),
        check=False,
    )

    assert result.returncode != 0
    assert "health.json profile update latest target does not match catalog" in result.stderr


def test_release_index_check_rejects_health_manifest_drift(tmp_path: Path) -> None:
    manifest_path = _write_release_manifest(tmp_path)
    dist = tmp_path / "target" / "release-channel"
    _run_admin(
        "assets",
        "channel",
        "build",
        "--manifest",
        f"file://{manifest_path}",
        "--assets-dir",
        str(manifest_path.parent),
        "--channel",
        "stable",
        "--out-dir",
        str(dist),
    )

    health_path = dist / "health.json"
    health = json.loads(health_path.read_text(encoding="utf-8"))
    health["updates"]["assets"]["manifest"] = "/assets/nightly/manifest.json"
    health_path.write_text(json.dumps(health, indent=2) + "\n", encoding="utf-8")

    result = _run_admin(
        "assets",
        "channel",
        "check",
        "--channel",
        "stable",
        "--dist",
        str(dist),
        check=False,
    )

    assert result.returncode != 0
    assert "health.json asset update manifest mismatch" in result.stderr


def test_release_index_check_rejects_missing_vm_obom_evidence(tmp_path: Path) -> None:
    manifest_path = _write_release_manifest(tmp_path)
    dist = tmp_path / "target" / "release-channel"
    _run_admin(
        "assets",
        "channel",
        "build",
        "--manifest",
        f"file://{manifest_path}",
        "--assets-dir",
        str(manifest_path.parent),
        "--channel",
        "stable",
        "--out-dir",
        str(dist),
    )

    health_path = dist / "health.json"
    health = json.loads(health_path.read_text(encoding="utf-8"))
    health["evidence"]["vm_oboms"] = []
    health_path.write_text(json.dumps(health, indent=2) + "\n", encoding="utf-8")

    result = _run_admin(
        "assets",
        "channel",
        "check",
        "--channel",
        "stable",
        "--dist",
        str(dist),
        check=False,
    )

    assert result.returncode != 0
    assert "health.json missing VM OBOM evidence" in result.stderr


def test_release_index_check_rejects_missing_host_sbom_evidence(tmp_path: Path) -> None:
    manifest_path = _write_release_manifest(tmp_path)
    dist = tmp_path / "target" / "release-channel"
    _run_admin(
        "assets",
        "channel",
        "build",
        "--manifest",
        f"file://{manifest_path}",
        "--assets-dir",
        str(manifest_path.parent),
        "--channel",
        "stable",
        "--out-dir",
        str(dist),
    )

    health_path = dist / "health.json"
    health = json.loads(health_path.read_text(encoding="utf-8"))
    health["evidence"]["host_sboms"] = []
    health_path.write_text(json.dumps(health, indent=2) + "\n", encoding="utf-8")

    result = _run_admin(
        "assets",
        "channel",
        "check",
        "--channel",
        "stable",
        "--dist",
        str(dist),
        check=False,
    )

    assert result.returncode != 0
    assert "health.json host SBOM evidence missing" in result.stderr


def test_release_index_check_rejects_host_binary_hash_drift(tmp_path: Path) -> None:
    manifest_path = _write_release_manifest(tmp_path)
    dist = tmp_path / "target" / "release-channel"
    _run_admin(
        "assets",
        "channel",
        "build",
        "--manifest",
        f"file://{manifest_path}",
        "--assets-dir",
        str(manifest_path.parent),
        "--channel",
        "stable",
        "--out-dir",
        str(dist),
    )

    health_path = dist / "health.json"
    health = json.loads(health_path.read_text(encoding="utf-8"))
    health["evidence"]["host_binary_files"][0]["sha256"] = "0" * 64
    health_path.write_text(json.dumps(health, indent=2) + "\n", encoding="utf-8")

    result = _run_admin(
        "assets",
        "channel",
        "check",
        "--channel",
        "stable",
        "--dist",
        str(dist),
        check=False,
    )

    assert result.returncode != 0
    assert "health.json host binary sha256 mismatch" in result.stderr
