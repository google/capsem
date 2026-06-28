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


def _write_release_manifest(
    root: Path,
    *,
    asset_version: str = "2030.0101.1",
    binary_version: str = "1.4.1234567890",
    date: str = "2030-01-01",
) -> Path:
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
            "current": asset_version,
            "releases": {
                asset_version: {
                    "date": date,
                    "deprecated": False,
                    "min_binary": "1.4.0",
                    "arches": {"arm64": files},
                }
            },
        },
        "binaries": {
            "current": binary_version,
            "releases": {
                binary_version: {
                    "date": date,
                    "deprecated": False,
                    "min_assets": asset_version,
                    "files": [
                        {
                            "name": f"Capsem-{binary_version}.pkg",
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


def test_release_index_generator_writes_split_cache_headers(tmp_path: Path) -> None:
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
        "--generated-at",
        "2030-01-01T00:00:00Z",
        "--json",
    )

    headers = (dist / "_headers").read_text(encoding="utf-8")
    assert "/\n  Cache-Control: no-cache, must-revalidate" in headers
    assert "/index.html\n  Cache-Control: no-cache, must-revalidate" in headers
    assert "/health.json\n  Cache-Control: no-cache, must-revalidate" in headers
    assert "/assets/stable/*\n  Cache-Control: no-cache, must-revalidate" in headers
    assert "/profiles/stable/*\n  Cache-Control: no-cache, must-revalidate" in headers
    assert "/assets/releases/*\n  Cache-Control: public, max-age=31536000, immutable" in headers
    assert "/profiles/releases/*\n  Cache-Control: public, max-age=31536000, immutable" in headers
    assert "/assets/*\n  Cache-Control: no-cache" not in headers
    assert "/profiles/*\n  Cache-Control: no-cache" not in headers


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
        "--generated-at",
        "2030-01-01T00:00:00Z",
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
    assert '<a href="/index.html">/index.html</a>' in index_html
    assert '<a href="/health.json">/health.json</a>' in index_html
    assert '<a href="/profiles/releases/profiles-2030.0101.1/catalog.json">' in index_html
    assert "/assets/releases/2030.0101.1/arm64-rootfs.erofs" in index_html
    assert "/assets/releases/2030.0101.1/arm64-obom.cdx.json" in index_html
    assert "capsem-sbom.spdx.json" in index_html
    assert "The fastest way to ship with AI securely." not in index_html

    headers = (dist / "_headers").read_text(encoding="utf-8")
    assert "/\n  Cache-Control: no-cache, must-revalidate" in headers
    assert "/health.json\n  Cache-Control: no-cache, must-revalidate" in headers
    assert "/assets/stable/*\n  Cache-Control: no-cache, must-revalidate" in headers
    assert "/profiles/stable/*\n  Cache-Control: no-cache, must-revalidate" in headers
    assert "/assets/releases/*\n  Cache-Control: public, max-age=31536000, immutable" in headers
    assert "/profiles/releases/*\n  Cache-Control: public, max-age=31536000, immutable" in headers
    assert "/assets/*\n  Cache-Control: no-cache" not in headers
    assert "/profiles/*\n  Cache-Control: no-cache" not in headers

    health = json.loads((dist / "health.json").read_text(encoding="utf-8"))
    assert health["schema"] == "capsem.assets_channel.health.v1"
    assert health["generated_at"] == "2030-01-01T00:00:00Z"
    assert health["urls"]["index"] == "/index.html"
    assert health["urls"]["health"] == "/health.json"
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
    assert health["urls"]["profile_catalog"] == catalog_url
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
    vm_asset_attestation = next(
        item
        for item in health["evidence"]["attestations"]
        if item["name"] == "github_attestations_vm_assets"
    )
    assert vm_asset_attestation["scope"] == "vm_assets"
    assert vm_asset_attestation["workflow"] == ".github/workflows/release-assets.yaml"
    assert "/assets/releases/2030.0101.1/arm64-rootfs.erofs" in vm_asset_attestation["subjects"]

    release_dir = dist / "assets" / "releases" / "2030.0101.1"
    assert (dist / "assets" / "stable" / "manifest.json").is_file()
    assert (release_dir / "arm64-vmlinuz").read_bytes() == b"kernel-arm64"
    assert (release_dir / "arm64-initrd.img").read_bytes() == b"initrd-arm64"
    assert (release_dir / "arm64-rootfs.erofs").read_bytes() == b"rootfs-arm64"
    assert (release_dir / "arm64-obom.cdx.json").is_file()

    _run_admin("assets", "channel", "check", "--channel", "stable", "--dist", str(dist))


def test_asset_release_updates_release_index_without_moving_binary_pointer(
    tmp_path: Path,
) -> None:
    manifest_path = _write_release_manifest(
        tmp_path,
        asset_version="2030.0102.1",
        binary_version="1.4.1234567890",
        date="2030-01-02",
    )
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
        "--generated-at",
        "2030-01-02T00:00:00Z",
    )

    health = json.loads((dist / "health.json").read_text(encoding="utf-8"))
    assert health["generated_at"] == "2030-01-02T00:00:00Z"
    assert health["current"] == {
        "binary": "1.4.1234567890",
        "assets": "2030.0102.1",
    }
    assert health["updates"]["binary"]["latest"] == "1.4.1234567890"
    assert health["updates"]["binary"]["current"] == "1.4.1234567890"
    assert health["updates"]["assets"]["latest"] == "2030.0102.1"
    assert health["updates"]["assets"]["current"] == "2030.0102.1"
    assert health["updates"]["assets"]["manifest"] == "/assets/stable/manifest.json"
    assert (
        health["assets"]["files"][0]["url"]
        == "/assets/releases/2030.0102.1/arm64-initrd.img"
    )
    assert (
        dist / "assets" / "releases" / "2030.0102.1" / "arm64-rootfs.erofs"
    ).read_bytes() == b"rootfs-arm64"
    assert (dist / "assets" / "stable" / "manifest.json").is_file()

    _run_admin("assets", "channel", "check", "--channel", "stable", "--dist", str(dist))


def test_asset_channel_deprecate_release_reports_history_without_moving_current(
    tmp_path: Path,
) -> None:
    manifest_path = _write_release_manifest(
        tmp_path,
        asset_version="2030.0102.1",
        binary_version="1.4.1234567890",
        date="2030-01-02",
    )
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    current_release = manifest["assets"]["releases"]["2030.0102.1"]
    deprecated_release = dict(current_release)
    deprecated_release["date"] = "2030-01-01"
    deprecated_release["deprecated"] = True
    deprecated_release["deprecated_date"] = "2030-01-03"
    manifest["assets"]["releases"]["2030.0101.1"] = deprecated_release
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
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
        "--generated-at",
        "2030-01-03T00:00:00Z",
    )

    index_html = (dist / "index.html").read_text(encoding="utf-8")
    assert "Asset Release History" in index_html
    assert "2030.0101.1" in index_html
    assert "deprecated" in index_html
    assert "2030-01-03" in index_html
    assert "2030.0102.1" in index_html

    health = json.loads((dist / "health.json").read_text(encoding="utf-8"))
    assert health["current"] == {
        "binary": "1.4.1234567890",
        "assets": "2030.0102.1",
    }
    releases = {release["version"]: release for release in health["asset_releases"]}
    assert releases["2030.0102.1"]["state"] == "current"
    assert releases["2030.0102.1"]["deprecated"] is False
    assert releases["2030.0101.1"]["state"] == "deprecated"
    assert releases["2030.0101.1"]["deprecated"] is True
    assert releases["2030.0101.1"]["deprecated_date"] == "2030-01-03"
    assert (
        dist / "assets" / "releases" / "2030.0102.1" / "arm64-rootfs.erofs"
    ).read_bytes() == b"rootfs-arm64"
    assert not (dist / "assets" / "releases" / "2030.0101.1").exists()

    channel_manifest = json.loads(
        (dist / "assets" / "stable" / "manifest.json").read_text(encoding="utf-8")
    )
    assert channel_manifest["assets"]["releases"]["2030.0101.1"]["deprecated"] is True

    _run_admin("assets", "channel", "check", "--channel", "stable", "--dist", str(dist))


def test_asset_release_index_workflow_deploys_generated_preview_only_after_asset_change() -> None:
    workflow = (PROJECT_ROOT / ".github/workflows/release-assets.yaml").read_text(
        encoding="utf-8"
    )
    assemble_channel = workflow.split("  assemble-channel:", maxsplit=1)[1].split(
        "  deploy-channel:", maxsplit=1
    )[0]
    deploy_channel = workflow.split("  deploy-channel:", maxsplit=1)[1]

    assert "cargo run -p capsem-admin -- manifest generate assets" in assemble_channel
    assert "scripts/check-asset-release-delta.py" in assemble_channel
    assert "cargo run -p capsem-admin -- assets channel build" in assemble_channel
    assert '--manifest "file://$PWD/assets/manifest.json"' in assemble_channel
    assert '--out-dir target/release-channel' in assemble_channel
    assert "cargo run -p capsem-admin -- assets channel check" in assemble_channel
    assert "name: asset-channel-preview" in assemble_channel
    assert "path: target/release-channel/" in assemble_channel
    assert "asset_changed: ${{ steps.asset-delta.outputs.changed }}" in assemble_channel
    assert "if: ${{ steps.asset-delta.outputs.changed == 'true' }}" in assemble_channel

    assert (
        "if: ${{ inputs.dry_run == false && needs.assemble-channel.outputs.asset_changed == 'true' }}"
        in deploy_channel
    )
    assert "uses: ./.github/workflows/release-channel.yaml" in deploy_channel
    assert "dist_artifact: asset-channel-preview" in deploy_channel


def test_release_assets_workflow_allows_first_channel_bootstrap() -> None:
    workflow = (PROJECT_ROOT / ".github/workflows/release-assets.yaml").read_text(
        encoding="utf-8"
    )
    assemble_channel = workflow.split("  assemble-channel:", maxsplit=1)[1].split(
        "  deploy-channel:", maxsplit=1
    )[0]

    assert "scripts/check-asset-release-delta.py" in assemble_channel
    assert (
        '--previous-manifest-url "https://release.capsem.org/assets/$CHANNEL/manifest.json"'
        in assemble_channel
    )
    assert "--allow-missing-previous" in assemble_channel


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


def test_release_index_check_rejects_profile_catalog_url_drift(tmp_path: Path) -> None:
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
    health["urls"]["profile_catalog"] = "/profiles/releases/stale/catalog.json"
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
    assert "health.json profile catalog URL mismatch" in result.stderr


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


def test_release_index_check_rejects_missing_vm_asset_attestation_evidence(
    tmp_path: Path,
) -> None:
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
    health["evidence"]["attestations"] = [
        item
        for item in health["evidence"]["attestations"]
        if item["name"] != "github_attestations_vm_assets"
    ]
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
    assert "health.json VM asset attestation evidence missing" in result.stderr


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


def test_binary_release_index_records_host_artifacts_without_changing_assets(
    tmp_path: Path,
) -> None:
    manifest_path = _write_release_manifest(tmp_path)
    before = json.loads(manifest_path.read_text(encoding="utf-8"))
    artifacts = tmp_path / "release-artifacts"
    artifacts.mkdir()
    pkg = artifacts / "Capsem-1.4.2234567890.pkg"
    deb = artifacts / "Capsem_1.4.2234567890_arm64.deb"
    sbom = artifacts / "capsem-sbom.spdx.json"
    pkg.write_bytes(b"pkg bytes v2")
    deb.write_bytes(b"deb bytes v2")
    sbom.write_bytes(b'{"spdxVersion":"SPDX-2.3","name":"capsem"}')

    result = _run_admin(
        "assets",
        "channel",
        "record-binary",
        "--manifest-path",
        str(manifest_path),
        "--version",
        "1.4.2234567890",
        "--date",
        "2030-02-03",
        "--artifact",
        str(pkg),
        "--artifact",
        str(deb),
        "--artifact",
        str(sbom),
        "--json",
    )

    report = json.loads(result.stdout)
    after = json.loads(manifest_path.read_text(encoding="utf-8"))
    assert report["schema"] == "capsem.admin.assets_channel_record_binary.v1"
    assert report["version"] == "1.4.2234567890"
    assert report["min_assets"] == before["assets"]["current"]
    assert after["assets"] == before["assets"]
    assert after["binaries"]["current"] == "1.4.2234567890"
    release = after["binaries"]["releases"]["1.4.2234567890"]
    assert release["min_assets"] == "2030.0101.1"
    assert release["version"] == "1.4.2234567890"
    assert release["date"] == "2030-02-03"
    files = {entry["name"]: entry for entry in release["files"]}
    assert files[pkg.name]["sha256"] == hashlib.sha256(b"pkg bytes v2").hexdigest()
    assert files[deb.name]["sha256"] == hashlib.sha256(b"deb bytes v2").hexdigest()
    assert files[sbom.name]["sha256"] == hashlib.sha256(sbom.read_bytes()).hexdigest()


def test_binary_release_profile_catalog_index_builds_release_site_without_rebuilding_vm_assets(
    tmp_path: Path,
) -> None:
    manifest_path = _write_release_manifest(tmp_path)
    profiles_dir = _write_profile_catalog(tmp_path)
    source_base = tmp_path / "published-assets" / "releases" / "2030.0101.1"
    source_base.mkdir(parents=True)
    for logical in ("vmlinuz", "initrd.img", "rootfs.erofs", "obom.cdx.json"):
        (source_base / f"arm64-{logical}").write_bytes(
            (tmp_path / "assets" / "arm64" / logical).read_bytes()
        )
    # A tag release runner must not need local VM build outputs.
    for local_asset in (tmp_path / "assets" / "arm64").iterdir():
        local_asset.unlink()

    dist = tmp_path / "target" / "release-channel"
    _run_admin(
        "assets",
        "channel",
        "build",
        "--manifest",
        f"file://{manifest_path}",
        "--assets-dir",
        str(tmp_path / "assets"),
        "--asset-source-base",
        f"file://{tmp_path / 'published-assets' / 'releases'}",
        "--profiles-dir",
        str(profiles_dir),
        "--channel",
        "stable",
        "--out-dir",
        str(dist),
    )

    health = json.loads((dist / "health.json").read_text(encoding="utf-8"))
    assert health["current"] == {
        "binary": "1.4.1234567890",
        "assets": "2030.0101.1",
    }
    assert health["evidence"]["host_binary_files"]
    assert health["evidence"]["host_sboms"]
    assert health["evidence"]["attestations"]
    catalog_url = "/profiles/releases/profiles-2030.0101.1/catalog.json"
    assert health["profiles"]["source"] == catalog_url
    assert health["updates"]["profiles"]["source"] == catalog_url
    assert health["profiles"]["hash"] == health["updates"]["profiles"]["hash"]
    catalog_path = dist / catalog_url.removeprefix("/")
    catalog = json.loads(catalog_path.read_text(encoding="utf-8"))
    assert catalog["schema"] == "capsem.profile_catalog.v1"
    assert catalog["revision"] == "profiles-2030.0101.1"
    assert catalog["compatibility"] == {
        "binary": "1.4.1234567890",
        "assets": "2030.0101.1",
        "min_binary": "1.4.0",
        "min_assets": "2030.0101.1",
        "requires_newer_binary": False,
        "requires_newer_assets": False,
    }
    assert "file://" not in catalog_path.read_text(encoding="utf-8")
    assert str(tmp_path) not in catalog_path.read_text(encoding="utf-8")
    assert (
        dist / "assets" / "releases" / "2030.0101.1" / "arm64-rootfs.erofs"
    ).read_bytes() == b"rootfs-arm64"
    _run_admin("assets", "channel", "check", "--channel", "stable", "--dist", str(dist))
