"""Black-box profile materialization checks for capsem-admin."""

from __future__ import annotations

import json
import os
import re
import subprocess
import tomllib
from pathlib import Path

from blake3 import blake3

PROJECT_ROOT = Path(__file__).resolve().parents[2]
ADMIN = PROJECT_ROOT / "target" / "debug" / "capsem-admin"
SOURCE_PROFILE = PROJECT_ROOT / "config" / "profiles" / "code" / "profile.toml"
SOURCE_PROFILE_DIR = SOURCE_PROFILE.parent


def _host_arch() -> str:
    return "arm64" if os.uname().machine == "arm64" else "x86_64"


def _ensure_admin_binary() -> None:
    admin_source = PROJECT_ROOT / "crates" / "capsem-admin" / "src" / "main.rs"
    if ADMIN.exists() and ADMIN.stat().st_mtime >= admin_source.stat().st_mtime:
        return
    subprocess.run(
        ["cargo", "build", "-p", "capsem-admin"],
        cwd=PROJECT_ROOT,
        check=True,
        capture_output=True,
        text=True,
        timeout=120,
    )


def _load_toml(path: Path) -> dict:
    return tomllib.loads(path.read_text())


def _write_asset(path: Path, data: bytes) -> dict[str, object]:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
    return {"hash": blake3(data).hexdigest(), "size": len(data)}


def _write_publishable_manifest(root: Path) -> Path:
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
        "software-inventory.json": _write_asset(
            arm64 / "software-inventory.json",
            b'{"schema":"capsem.profile_software_inventory.v1","architecture":"arm64","packages":[]}',
        ),
    }
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
                }
            },
        },
    }
    manifest_path = assets / "manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return manifest_path


def _write_local_url_profile_catalog(root: Path) -> Path:
    profiles_dir = root / "config" / "profiles"
    profile_dir = profiles_dir / "code"
    profile_dir.mkdir(parents=True, exist_ok=True)
    (profile_dir / "profile.toml").write_text(
        f"""
id = "code"
name = "Code"
description = "Profile catalog fixture with local source URLs."
revision = "profiles-2030.0101.1"
refresh_policy = "24h"

[assets]
format = "profile-assets.v1"
refresh_policy = "on_profile_refresh"

[assets.arch.arm64.kernel]
name = "vmlinuz"
url = "{(root / "assets" / "arm64" / "vmlinuz").resolve().as_uri()}"

[assets.arch.arm64.initrd]
name = "initrd.img"
url = "{(root / "assets" / "arm64" / "initrd.img").resolve().as_uri()}"

[assets.arch.arm64.rootfs]
name = "rootfs.erofs"
url = "{(root / "assets" / "arm64" / "rootfs.erofs").resolve().as_uri()}"
""".strip()
        + "\n",
        encoding="utf-8",
    )
    return profiles_dir


def test_profile_materialize_generates_pins_without_mutating_source(tmp_path: Path) -> None:
    _ensure_admin_binary()
    arch = _host_arch()
    output_root = tmp_path / "target-config"
    result = subprocess.run(
        [
            str(ADMIN),
            "profile",
            "materialize",
            "--profile",
            str(SOURCE_PROFILE),
            "--config-root",
            "config",
            "--manifest",
            (PROJECT_ROOT / "assets/manifest.json").resolve().as_uri(),
            "--assets-dir",
            "assets",
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
        timeout=30,
    )

    assert result.returncode == 0, (
        f"capsem-admin profile materialize failed:\nstdout={result.stdout}\nstderr={result.stderr}"
    )
    report = json.loads(result.stdout)
    assert report["schema"] == "capsem.admin.profile_materialize.v1"
    assert report["ok"] is True
    assert report["profile_id"] == "code"
    assert report["profile_path"] == str(output_root / "profiles" / "code" / "profile.toml")
    assert report["manifest"] == str(output_root / "assets" / "manifest.json")
    assert {asset["logical_name"] for asset in report["materialized_assets"]} == {
        "vmlinuz",
        "initrd.img",
        "rootfs.erofs",
    }
    assert {asset["arch"] for asset in report["materialized_assets"]} == {arch}
    assert len(report["materialized_obom"]) == 1
    assert report["materialized_obom"][0]["scope"] == "base_image"

    source_text = SOURCE_PROFILE.read_text()
    assert not re.search(r"(?m)^\s*(hash|size)\s=", source_text)

    generated_profile = output_root / "profiles" / "code" / "profile.toml"
    generated = _load_toml(generated_profile)
    source = _load_toml(SOURCE_PROFILE)
    assert generated["id"] == source["id"]
    assert generated["name"] == source["name"]
    assert generated["description"] == source["description"]
    assert set(generated["assets"]["arch"]) == {arch}

    arch_assets = generated["assets"]["arch"][arch]
    for key in ("kernel", "initrd", "rootfs"):
        descriptor = arch_assets[key]
        assert descriptor["url"].startswith("file://")
        assert re.fullmatch(r"blake3:[0-9a-f]{64}", descriptor["hash"])
        assert descriptor["size"] > 0

    for file_key in (
        "enforcement",
        "detection",
        "mcp",
        "apt_packages",
        "python_requirements",
        "npm_packages",
        "build",
        "tips",
        "root_manifest",
    ):
        descriptor = generated["files"][file_key]
        assert re.fullmatch(r"blake3:[0-9a-f]{64}", descriptor["hash"])
        assert descriptor["size"] > 0
        source_file = PROJECT_ROOT / "config" / source["files"][file_key]["path"]
        generated_file = output_root / descriptor["path"]
        assert generated_file.read_bytes() == source_file.read_bytes()

    assert (output_root / "assets" / "manifest.json").read_bytes() == (
        PROJECT_ROOT / "assets" / "manifest.json"
    ).read_bytes()
    assert not (output_root / "admin").exists()
    assert not (output_root / "skills").exists()


def test_assets_channel_profile_catalog_is_publishable_not_local(tmp_path: Path) -> None:
    _ensure_admin_binary()
    manifest_path = _write_publishable_manifest(tmp_path)
    profiles_dir = _write_local_url_profile_catalog(tmp_path)
    dist = tmp_path / "target" / "release-channel"

    result = subprocess.run(
        [
            str(ADMIN),
            "assets",
            "channel",
            "build",
            "--manifest",
            manifest_path.resolve().as_uri(),
            "--assets-dir",
            str(manifest_path.parent),
            "--profiles-dir",
            str(profiles_dir),
            "--channel",
            "stable",
            "--out-dir",
            str(dist),
            "--json",
        ],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        timeout=30,
    )

    assert result.returncode == 0, (
        f"capsem-admin assets channel build failed:\nstdout={result.stdout}\nstderr={result.stderr}"
    )
    health = json.loads((dist / "health.json").read_text(encoding="utf-8"))
    assert health["profiles"]["source"] == "manifest.profiles"
    assert health["updates"]["profiles"]["source"] == "manifest.profiles"
    assert "profile_catalog" not in health["urls"]

    manifest_path = dist / "assets" / "stable" / "manifest.json"
    manifest_text = manifest_path.read_text(encoding="utf-8")
    assert "file://" not in manifest_text
    assert str(tmp_path) not in manifest_text
    assert "/assets/releases/2030.0101.1/arm64-vmlinuz" in manifest_text
    assert (
        "/assets/releases/2030.0101.1/arm64-obom.cdx.json" in manifest_text
    )

    manifest = json.loads(manifest_text)
    assert "profile_catalog" not in manifest
    profile = manifest["profiles"]["code"]
    arm64 = profile["architectures"][0]
    kernel = next(item for item in arm64["images"] if item["kind"] == "kernel")
    assert kernel["digest"]["blake3"]
    assert kernel["bytes"] > 0
    obom = next(item for item in arm64["evidence"] if item["kind"] == "obom")
    assert obom["url"] == (
        "/assets/releases/2030.0101.1/arm64-obom.cdx.json"
    )


def test_profile_materialize_rejects_bare_manifest_path(tmp_path: Path) -> None:
    _ensure_admin_binary()
    output_root = tmp_path / "target-config"
    manifest_path = (PROJECT_ROOT / "assets/manifest.json").resolve()

    result = subprocess.run(
        [
            str(ADMIN),
            "profile",
            "materialize",
            "--profile",
            str(SOURCE_PROFILE),
            "--config-root",
            "config",
            "--manifest",
            str(manifest_path),
            "--assets-dir",
            "assets",
            "--output-root",
            str(output_root),
            "--arch",
            "arm64",
        ],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        timeout=30,
    )

    assert result.returncode != 0
    err = result.stdout + result.stderr
    assert "manifest must be a URL" in err, err
    assert "file:///absolute/path" in err, err


def test_checked_in_source_profiles_keep_generation_hashes_out_of_profile_toml() -> None:
    offenders = []
    for profile_path in sorted((PROJECT_ROOT / "config" / "profiles").glob("*/profile.toml")):
        if re.search(r"(?m)^\s*(hash|size)\s=", profile_path.read_text()):
            offenders.append(str(profile_path.relative_to(PROJECT_ROOT)))

    assert offenders == []
