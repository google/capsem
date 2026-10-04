"""Release-channel index generator contract tests."""

from __future__ import annotations

import gzip
import hashlib
import io
import json
import subprocess
import sys
import tarfile
from pathlib import Path

from blake3 import blake3
from helpers.release_site import build_release_channel_site

PROJECT_ROOT = Path(__file__).resolve().parents[2]
SOURCE_COMMIT = "0" * 40


def _rootfs_obom_bytes(architecture: str) -> bytes:
    return (
        json.dumps(
            {
                "bomFormat": "CycloneDX",
                "metadata": {
                    "component": {
                        "type": "operating-system",
                        "name": f"capsem-rootfs-{architecture}",
                        "version": "guest-rootfs",
                        "properties": [
                            {"name": "capsem:evidence:scope", "value": "exported-rootfs"},
                            {"name": "capsem:guest:architecture", "value": architecture},
                        ],
                    },
                    "tools": {"components": [{"name": "cdxgen", "version": "12.7.0"}]},
                },
                "components": [
                    {
                        "type": "library",
                        "name": "apt",
                        "version": "2.6.1",
                        "purl": "pkg:deb/debian/apt@2.6.1?distro=debian-12",
                    }
                ],
            },
            sort_keys=True,
        )
        + "\n"
    ).encode()


def _run_admin(*args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(
        ["cargo", "run", "-p", "capsem-admin", "--quiet", "--", *args],
        cwd=PROJECT_ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    if check and result.returncode != 0:
        raise AssertionError(
            f"capsem-admin {' '.join(args)} failed\n"
            f"stdout:\n{result.stdout}\n"
            f"stderr:\n{result.stderr}"
        )
    if (
        result.returncode == 0
        and len(args) >= 3
        and args[:3] == ("assets", "channel", "build")
        and "--out-dir" in args
    ):
        out_dir = Path(args[args.index("--out-dir") + 1])
        _build_release_site(out_dir)
    return result


def _build_release_site(dist: Path) -> None:
    build_release_channel_site(dist)


def _write_asset(path: Path, data: bytes) -> dict[str, object]:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
    return {"hash": blake3(data).hexdigest(), "size": len(data)}


def _write_minimal_deb(path: Path, executable_name: str = "capsem-app") -> bytes:
    executable = b"#!/bin/sh\nexit 0\n"
    control = (
        b"Package: capsem\n"
        b"Version: 1.4.2\n"
        b"Architecture: arm64\n"
        b"Maintainer: Capsem <release@capsem.org>\n"
        b"Description: Capsem contract-test package\n"
    )
    data_tar = io.BytesIO()
    with (
        gzip.GzipFile(fileobj=data_tar, mode="wb", mtime=0) as gz,
        tarfile.open(fileobj=gz, mode="w") as tar,
    ):
        info = tarfile.TarInfo(f"usr/bin/{executable_name}")
        info.mode = 0o755
        info.size = len(executable)
        info.mtime = 0
        tar.addfile(info, io.BytesIO(executable))
    control_tar = io.BytesIO()
    with (
        gzip.GzipFile(fileobj=control_tar, mode="wb", mtime=0) as gz,
        tarfile.open(fileobj=gz, mode="w") as tar,
    ):
        info = tarfile.TarInfo("./control")
        info.mode = 0o644
        info.size = len(control)
        info.mtime = 0
        tar.addfile(info, io.BytesIO(control))
    deb = (
        b"!<arch>\n"
        + _ar_member("debian-binary", b"2.0\n")
        + _ar_member("control.tar.gz", control_tar.getvalue())
        + _ar_member("data.tar.gz", data_tar.getvalue())
    )
    path.write_bytes(deb)
    return deb


def _write_minimal_pkg(path: Path) -> bytes:
    executable = b"#!/bin/sh\nexit 0\n"
    installed_path = "Applications/Capsem.app/Contents/MacOS/capsem-app"
    if sys.platform == "darwin":
        root = path.with_suffix(".root")
        payload = root / installed_path
        payload.parent.mkdir(parents=True)
        payload.write_bytes(executable)
        payload.chmod(0o755)
        subprocess.run(
            [
                "pkgbuild",
                "--root",
                str(root),
                "--identifier",
                "org.capsem.test.fixture",
                "--version",
                "1.4.2",
                str(path),
            ],
            check=True,
            capture_output=True,
            text=True,
        )
    else:
        with tarfile.open(path, mode="w:gz") as tar:
            info = tarfile.TarInfo(f"capsem.pkg/Payload/{installed_path}")
            info.mode = 0o755
            info.size = len(executable)
            info.mtime = 0
            tar.addfile(info, io.BytesIO(executable))
    return path.read_bytes()


def _ar_member(name: str, data: bytes) -> bytes:
    header = (f"{name + '/':<16}{0:<12}{0:<6}{0:<6}{0o100644:<8}{len(data):<10}`\n").encode("ascii")
    return header + data + (b"\n" if len(data) % 2 else b"")


def _write_release_manifest(
    root: Path,
    *,
    asset_version: str = "2030.0101.1",
    binary_version: str = "1.4.1",
    date: str = "2030-01-01",
    include_binary_files: bool = True,
    include_x86_64: bool = False,
) -> Path:
    assets = root / "assets"
    arm64 = assets / "arm64"
    arm64_files = {
        "vmlinuz": _write_asset(arm64 / "vmlinuz", b"kernel-arm64"),
        "initrd.img": _write_asset(arm64 / "initrd.img", b"initrd-arm64"),
        "rootfs.erofs": _write_asset(arm64 / "rootfs.erofs", b"rootfs-arm64"),
        "abom.cdx.json": _write_asset(
            arm64 / "abom.cdx.json",
            b'{"bomFormat":"CycloneDX","metadata":{"tools":[{"name":"cdxgen"}]}}',
        ),
        "obom.cdx.json": _write_asset(
            arm64 / "obom.cdx.json",
            _rootfs_obom_bytes("arm64"),
        ),
        "software-inventory.json": _write_asset(
            arm64 / "software-inventory.json",
            json.dumps(
                {
                    "schema": "capsem.profile_software_inventory.v1",
                    "architecture": "arm64",
                    "packages": [
                        {
                            "name": "zstd",
                            "version": "1.5.6",
                            "source": "apt",
                            "architecture": "arm64",
                        }
                    ],
                }
            ).encode("utf-8"),
        ),
    }
    arches = {"arm64": arm64_files}
    if include_x86_64:
        x86_64 = assets / "x86_64"
        arches["x86_64"] = {
            "vmlinuz": _write_asset(x86_64 / "vmlinuz", b"kernel-x86_64"),
            "initrd.img": _write_asset(x86_64 / "initrd.img", b"initrd-x86_64"),
            "rootfs.erofs": _write_asset(x86_64 / "rootfs.erofs", b"rootfs-x86_64"),
            "abom.cdx.json": _write_asset(
                x86_64 / "abom.cdx.json",
                b'{"bomFormat":"CycloneDX","metadata":{"tools":[{"name":"cdxgen"}]}}',
            ),
            "obom.cdx.json": _write_asset(
                x86_64 / "obom.cdx.json",
                _rootfs_obom_bytes("x86_64"),
            ),
            "software-inventory.json": _write_asset(
                x86_64 / "software-inventory.json",
                json.dumps(
                    {
                        "schema": "capsem.profile_software_inventory.v1",
                        "architecture": "x86_64",
                        "packages": [
                            {
                                "name": "zstd",
                                "version": "1.5.6",
                                "source": "apt",
                                "architecture": "x86_64",
                            }
                        ],
                    }
                ).encode("utf-8"),
            ),
        }
    pkg = b"pkg bytes"
    sbom = b'{"spdxVersion":"SPDX-2.3"}'
    package_sbom = b'{"spdxVersion":"SPDX-2.3","package":"capsem-1-4-1-pkg"}'
    capsem_app = b"capsem app executable"
    capsem_tray = b"capsem tray executable"
    binary_release = {
        "date": date,
        "deprecated": False,
        "min_assets": asset_version,
    }
    if include_binary_files:
        binary_release["files"] = [
            {
                "name": f"Capsem-{binary_version}.pkg",
                "size": len(pkg),
                "sha256": hashlib.sha256(pkg).hexdigest(),
                "blake3": blake3(pkg).hexdigest(),
                "binaries": [
                    {
                        "name": "capsem-app",
                        "description": "Capsem desktop application executable",
                        "installed_path": "/Applications/Capsem.app/Contents/MacOS/capsem-app",
                        "size": len(capsem_app),
                        "sha256": hashlib.sha256(capsem_app).hexdigest(),
                        "blake3": blake3(capsem_app).hexdigest(),
                        "sbom_component_ref": "SPDXRef-File-capsem-app",
                    },
                    {
                        "name": "capsem-tray",
                        "description": "Capsem tray companion executable",
                        "installed_path": "/Applications/Capsem.app/Contents/MacOS/capsem-tray",
                        "size": len(capsem_tray),
                        "sha256": hashlib.sha256(capsem_tray).hexdigest(),
                        "blake3": blake3(capsem_tray).hexdigest(),
                        "sbom_component_ref": "SPDXRef-File-capsem-tray",
                    },
                ],
            },
            {
                "name": "capsem-sbom.spdx.json",
                "size": len(sbom),
                "sha256": hashlib.sha256(sbom).hexdigest(),
                "blake3": blake3(sbom).hexdigest(),
            },
            {
                "name": "capsem-1-4-1-pkg-sbom.spdx.json",
                "size": len(package_sbom),
                "sha256": hashlib.sha256(package_sbom).hexdigest(),
                "blake3": blake3(package_sbom).hexdigest(),
            },
        ]

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
                    "arches": arches,
                }
            },
        },
        "binaries": {
            "current": binary_version,
            "releases": {binary_version: binary_release},
        },
    }
    manifest_path = assets / "manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return manifest_path


def _hydrate_asset_sha256_in_manifest(manifest_path: Path) -> None:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    assets_dir = manifest_path.parent
    for release in manifest["assets"]["releases"].values():
        for arch, assets in release["arches"].items():
            for logical_name, entry in assets.items():
                entry["sha256"] = hashlib.sha256(
                    (assets_dir / arch / logical_name).read_bytes()
                ).hexdigest()
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


# The runtime revision is the asset release the manifest names current
# (`assets.current`): one kernel/initrd/rootfs set per architecture, with no
# profile input. Deliberately not the workspace version: a fixture equal to the
# real one hides whether the code under test depends on it.
RUNTIME_REVISION = "2030.0101.1"


def test_release_index_generator_writes_split_cache_headers(tmp_path: Path) -> None:
    manifest_path = _write_release_manifest(tmp_path)
    dist = tmp_path / "cache" / "target" / "release-channel"

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
        "--generated-at",
        "2030-01-01T00:00:00Z",
        "--json",
    )

    headers = (dist / "_headers").read_text(encoding="utf-8")
    assert "/\n  Cache-Control: no-cache, must-revalidate" in headers
    assert "/index.html\n  Cache-Control: no-cache, must-revalidate" in headers
    assert "/health.json\n  Cache-Control: no-cache, must-revalidate" in headers
    assert "/assets/stable/*\n  Cache-Control: no-cache, must-revalidate" in headers
    assert "/assets/releases/*\n  Cache-Control: public, max-age=31536000, immutable" in headers
    assert "/runtime/releases/*\n  Cache-Control: public, max-age=31536000, immutable" in headers
    assert "/assets/*\n  Cache-Control: no-cache" not in headers
    assert "/profiles/" not in headers


def test_release_index_generator_builds_human_and_machine_outputs(tmp_path: Path) -> None:
    manifest_path = _write_release_manifest(tmp_path, include_x86_64=True)
    dist = tmp_path / "cache" / "target" / "release-channel"

    result = _run_admin(
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
        "--generated-at",
        "2030-01-01T00:00:00Z",
        "--json",
    )

    report = json.loads(result.stdout)
    assert report["schema"] == "capsem.admin.assets_channel_build.v1"
    assert report["channel"] == "stable"
    assert report["human_site_source"] == "release-site"
    assert "index_html" not in report
    assert report["manifest"] == str(dist / "assets" / "stable" / "manifest.json")
    assert report["copied_assets"] == 12

    index_html = (dist / "index.html").read_text(encoding="utf-8")
    assert "Capsem Release Channels" in index_html
    assert "Stable" in index_html
    assert "Manifest revision" in index_html
    channel_html = (dist / "channels" / "stable" / "index.html").read_text(encoding="utf-8")
    assert "Current Manifest" in channel_html
    assert "Manifest URL" in channel_html
    assert "Capsem Packages" in channel_html
    assert RUNTIME_REVISION in channel_html
    assert "SBOM" in channel_html
    assert "Realm Discipline" not in index_html
    assert 'href="/channels.json"' in index_html
    assert 'href="/assets/stable/manifest.json"' in channel_html
    assert "/assets/stable/manifest.json" in channel_html
    assert "Capsem-1.4.1.pkg" in channel_html
    assert "capsem-1-4-1-pkg-sbom.spdx.json" in channel_html
    assert "The fastest way to ship with AI securely." not in index_html
    assert not (dist / "channels" / "stable" / "profiles").exists()
    assert not (dist / "profiles").exists()

    channel_manifest = json.loads(
        (dist / "assets" / "stable" / "manifest.json").read_text(encoding="utf-8")
    )
    assert "profiles" not in channel_manifest
    runtime = channel_manifest["runtime"]
    assert runtime["revision"] == RUNTIME_REVISION
    assert runtime["status"] == "current"
    assert runtime["min_capsem_version"] == "1.4.0"
    assert {"id", "name", "description", "version"}.isdisjoint(runtime)
    assert [row["architecture"] for row in runtime["architectures"]] == ["arm64", "x86_64"]
    for row in runtime["architectures"]:
        arch = row["architecture"]
        assert "config" not in row
        assert row["image_revision"] == RUNTIME_REVISION
        assert row["package_inventory_revision"] == RUNTIME_REVISION
        assert {image["kind"] for image in row["images"]} == {"kernel", "initrd", "rootfs"}
        for record in (*row["images"], *row["evidence"]):
            assert record["url"].startswith(
                f"/runtime/releases/stable/{RUNTIME_REVISION}/{arch}/"
            ), record
            assert (dist / record["url"].lstrip("/")).is_file(), record
    assert (
        dist / "runtime" / "releases" / "stable" / RUNTIME_REVISION / "arm64" / "rootfs.erofs"
    ).read_bytes() == b"rootfs-arm64"

    headers = (dist / "_headers").read_text(encoding="utf-8")
    assert "/\n  Cache-Control: no-cache, must-revalidate" in headers
    assert "/health.json\n  Cache-Control: no-cache, must-revalidate" in headers
    assert "/assets/stable/*\n  Cache-Control: no-cache, must-revalidate" in headers
    assert "/assets/releases/*\n  Cache-Control: public, max-age=31536000, immutable" in headers
    assert "/runtime/releases/*\n  Cache-Control: public, max-age=31536000, immutable" in headers
    assert "/assets/*\n  Cache-Control: no-cache" not in headers
    assert "/profiles/" not in headers

    health = json.loads((dist / "health.json").read_text(encoding="utf-8"))
    assert health["schema"] == "capsem.assets_channel.health.v1"
    assert health["generated_at"] == "2030-01-01T00:00:00Z"
    assert health["urls"]["index"] == "/index.html"
    assert health["urls"]["health"] == "/health.json"
    assert health["urls"]["manifest"] == "/assets/stable/manifest.json"
    assert health["urls"]["asset_base"] == "/assets/releases"
    assert health["current"] == {
        "binary": "1.4.1",
        "assets": "2030.0101.1",
    }
    assert health["updates"]["binary"]["latest"] == "1.4.1"
    assert health["updates"]["assets"]["manifest"] == "/assets/stable/manifest.json"
    assert "profiles" not in health
    assert "profiles" not in health["updates"]
    assert "profile_catalog" not in health["urls"]
    assert health["runtime"]["revision"] == RUNTIME_REVISION
    assert health["runtime"]["state"] == "current"
    assert health["runtime"]["source"] == "manifest.runtime"
    assert health["runtime"]["min_binary"] == "1.4.0"
    assert health["runtime"]["architectures"] == ["arm64", "x86_64"]
    assert health["updates"]["runtime"] == {
        "latest": RUNTIME_REVISION,
        "current": RUNTIME_REVISION,
        "state": "current",
        "source": "manifest.runtime",
    }
    assert "images" not in health["updates"]
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
    assert vm_asset_attestation["predicate_url"] == (
        "/assets/releases/2030.0101.1/arm64-obom.cdx.json"
    )
    assert "/assets/releases/2030.0101.1/arm64-rootfs.erofs" in vm_asset_attestation["subjects"]

    release_dir = dist / "assets" / "releases" / "2030.0101.1"
    assert (dist / "assets" / "stable" / "manifest.json").is_file()
    assert (release_dir / "arm64-vmlinuz").read_bytes() == b"kernel-arm64"
    assert (release_dir / "arm64-initrd.img").read_bytes() == b"initrd-arm64"
    assert (release_dir / "arm64-rootfs.erofs").read_bytes() == b"rootfs-arm64"
    assert (release_dir / "arm64-obom.cdx.json").is_file()

    _run_admin("assets", "channel", "check", "--channel", "stable", "--dist", str(dist))


def test_release_index_bootstraps_before_binary_evidence_exists(tmp_path: Path) -> None:
    manifest_path = _write_release_manifest(tmp_path, include_binary_files=False)
    dist = tmp_path / "cache" / "target" / "release-channel"

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
        "--generated-at",
        "2030-01-01T00:00:00Z",
        "--json",
    )

    health = json.loads((dist / "health.json").read_text(encoding="utf-8"))
    assert health["current"] == {
        "binary": "1.4.1",
        "assets": "2030.0101.1",
    }
    assert health["evidence"]["host_binary_files"] == []
    assert health["evidence"]["host_sboms"] == []
    assert all(
        item["name"] != "github_attestations_host" for item in health["evidence"]["attestations"]
    )
    assert any(
        item["name"] == "github_attestations_vm_assets"
        for item in health["evidence"]["attestations"]
    )
    assert health["runtime"]["min_binary"] == "1.4.0"

    _run_admin("assets", "channel", "check", "--channel", "stable", "--dist", str(dist))


def test_asset_release_updates_release_index_without_moving_binary_pointer(
    tmp_path: Path,
) -> None:
    manifest_path = _write_release_manifest(
        tmp_path,
        asset_version="2030.0102.1",
        binary_version="1.4.1",
        date="2030-01-02",
    )
    dist = tmp_path / "cache" / "target" / "release-channel"

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
        "--generated-at",
        "2030-01-02T00:00:00Z",
    )

    health = json.loads((dist / "health.json").read_text(encoding="utf-8"))
    assert health["generated_at"] == "2030-01-02T00:00:00Z"
    assert health["current"] == {
        "binary": "1.4.1",
        "assets": "2030.0102.1",
    }
    assert health["updates"]["binary"]["latest"] == "1.4.1"
    assert health["updates"]["binary"]["current"] == "1.4.1"
    assert health["updates"]["assets"]["latest"] == "2030.0102.1"
    assert health["updates"]["assets"]["current"] == "2030.0102.1"
    assert health["updates"]["assets"]["manifest"] == "/assets/stable/manifest.json"
    initrd_file = next(
        item for item in health["assets"]["files"] if item["logical_name"] == "initrd.img"
    )
    assert initrd_file["url"] == "/assets/releases/2030.0102.1/arm64-initrd.img"
    assert health["runtime"]["revision"] == "2030.0102.1"
    assert health["updates"]["runtime"]["latest"] == "2030.0102.1"
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
        binary_version="1.4.1",
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
    dist = tmp_path / "cache" / "target" / "release-channel"

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
        "--generated-at",
        "2030-01-03T00:00:00Z",
    )

    health = json.loads((dist / "health.json").read_text(encoding="utf-8"))
    assert health["current"] == {
        "binary": "1.4.1",
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
    assert "assets" not in channel_manifest
    assert channel_manifest["runtime"]["revision"] == "2030.0102.1"

    _run_admin("assets", "channel", "check", "--channel", "stable", "--dist", str(dist))


def _runtime_release_jobs() -> tuple[str, str, str, str, str]:
    workflow = (PROJECT_ROOT / ".github/workflows/release-assets.yaml").read_text(encoding="utf-8")
    resolve = workflow.split("  resolve-current-binary:", maxsplit=1)[1].split(
        "  cloudflare-release-site-preflight:", maxsplit=1
    )[0]
    pairing = workflow.split("  test-runtime-pairing:", maxsplit=1)[1].split(
        "  author-runtime-release:", maxsplit=1
    )[0]
    author = workflow.split("  author-runtime-release:", maxsplit=1)[1].split(
        "  publish-runtime-release:", maxsplit=1
    )[0]
    publish = workflow.split("  publish-runtime-release:", maxsplit=1)[1].split(
        "  deploy-channel:", maxsplit=1
    )[0]
    deploy_channel = workflow.split("  deploy-channel:", maxsplit=1)[1]
    return resolve, pairing, author, publish, deploy_channel


def test_runtime_release_deploys_generated_preview_only_when_activation_ready() -> None:
    resolve, pairing, author, publish, deploy_channel = _runtime_release_jobs()

    assert "cargo run -p capsem-admin -- validate" in resolve
    assert "build_system/scripts/release/check-runtime-release-delta.py" in resolve
    assert "release_needed: ${{ steps.runtime-delta.outputs.release_needed }}" in resolve
    assert "cargo run -p capsem-admin -- manifest generate cache/target/assets" in author
    assert '--version "$RUNTIME_REVISION"' in author
    assert "cargo run -p capsem-admin -- release" in author
    assert '--runtime-revision "$RUNTIME_REVISION"' in author
    assert "--profile" not in author
    assert "build_system/scripts/release/build-complete-release-channel.py" in author
    assert '--channel-source "$CHANNEL=file://$PWD/cache/target/assets/manifest.json"' in author
    assert '--primary-channel "$CHANNEL"' in author
    assert "--allow-mirror-missing" in author
    assert '--asset-source-base "$ASSET_BASE"' in author
    assert "releases/download/$PUBLICATION_IDENTITY" in author
    assert "--manifest-path cache/target/source-channel/manifest.json" in author
    assert "--out-dir cache/target/runtime-candidate" in author
    assert "product_compatible: ${{ steps.author-release.outputs.product_compatible }}" in author
    assert "functional_ready: ${{ needs.resolve-current-binary.outputs.functional_ready }}" in author
    assert "activation_ready: ${{ steps.author-release.outputs.activation_ready }}" in author

    # The activation branch moved inside `qualify-assets`, so the lane passes
    # the flag once instead of guarding three steps with it.
    assert "outputs.activation_ready" in pairing
    assert (
        pairing.count("if: ${{ needs.author-runtime-release.outputs.activation_ready == 'true' }}")
        == 0
    )

    assert "needs: [author-runtime-release, test-runtime-pairing" in publish
    assert "build_system/scripts/release/build-complete-release-channel.py" in publish
    assert "--out-dir cache/target/release/distribution" in publish
    assert "name: asset-channel-preview" in publish
    assert "path: cache/target/release/distribution/" in publish
    assert "if: ${{ needs.author-runtime-release.outputs.activation_ready == 'true' }}" in publish

    assert (
        "if: ${{ inputs.dry_run == false && "
        "needs.publish-runtime-release.outputs.activation_ready == 'true' }}" in deploy_channel
    )
    assert "uses: ./.github/workflows/release-channel.yaml" in deploy_channel
    assert "dist_artifact: asset-channel-preview" in deploy_channel


def test_runtime_release_publishes_deferred_assets_but_withholds_channel_deploy() -> None:
    _, pairing, author, publish, deploy_channel = _runtime_release_jobs()
    reusable_fast_gate = (PROJECT_ROOT / ".github/workflows/fast-gate.yaml").read_text(
        encoding="utf-8"
    )

    assert "cargo run -p capsem-admin -- release" in author
    assert "Qualify the runtime assets" in pairing
    assert "Run the complete fast gate" in reusable_fast_gate
    assert "run: just fast-test" in reusable_fast_gate
    assert (
        "run: uv run --project build_system --frozen capsem-gate test-release-contracts"
        in reusable_fast_gate
    )
    assert "Run shared release contracts" not in pairing
    # One verb owning both shapes; the lane decides which from this flag.
    qualify = pairing.split("- name: Qualify the runtime assets", maxsplit=1)[1].split(
        "- name: Record deferred runtime staging boundary", maxsplit=1
    )[0]
    assert "just qualify-assets" in qualify
    assert "outputs.activation_ready" in qualify
    assert "needs.author-runtime-release.outputs.activation_ready" in qualify
    assert "activation-ready runtime cannot defer complete pairing gates" in pairing
    assert "complete functional release binary cohort" in pairing
    assert "needs.author-runtime-release.outputs.activation_ready != 'true'" in pairing
    assert "Publish immutable GitHub runtime release" in publish
    immutable_release = publish.split(
        "- name: Publish immutable GitHub runtime release", maxsplit=1
    )[1].split("- uses: actions/upload-artifact@", maxsplit=1)[0]
    assert "outputs.activation_ready" not in immutable_release
    assert "needs.publish-runtime-release.outputs.activation_ready == 'true'" in deploy_channel


def test_release_index_check_rejects_runtime_index_drift(tmp_path: Path) -> None:
    manifest_path = _write_release_manifest(tmp_path)
    dist = tmp_path / "cache" / "target" / "release-channel"
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
    health["updates"]["runtime"]["latest"] = "2030.0101.0"
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
    assert (
        "health.json runtime update latest target does not match manifest runtime" in result.stderr
    )


def test_release_index_check_rejects_stale_human_index_state(tmp_path: Path) -> None:
    manifest_path = _write_release_manifest(tmp_path)
    dist = tmp_path / "cache" / "target" / "release-channel"
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
        "--generated-at",
        "2030-01-01T00:00:00Z",
    )

    index_path = dist / "index.html"
    index_html = index_path.read_text(encoding="utf-8")
    manifest_version = json.loads(
        (dist / "assets" / "stable" / "manifest.json").read_text(encoding="utf-8")
    )["version"]
    index_html = index_html.replace(manifest_version, "1.5.0-stale.20300101")
    index_path.write_text(index_html, encoding="utf-8")

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
    assert f"asset channel index missing manifest version {manifest_version}" in result.stderr


def test_release_index_check_rejects_health_manifest_drift(tmp_path: Path) -> None:
    manifest_path = _write_release_manifest(tmp_path)
    dist = tmp_path / "cache" / "target" / "release-channel"
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


def test_release_index_check_rejects_runtime_content_drift(tmp_path: Path) -> None:
    manifest_path = _write_release_manifest(tmp_path)
    dist = tmp_path / "cache" / "target" / "release-channel"
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

    manifest_path = dist / "assets" / "stable" / "manifest.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    manifest["runtime"]["revision"] = "2030.0101.0"
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")

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
    assert "channels.json manifest sha256 mismatch" in result.stderr


def test_release_index_check_rejects_missing_vm_obom_evidence(tmp_path: Path) -> None:
    manifest_path = _write_release_manifest(tmp_path)
    dist = tmp_path / "cache" / "target" / "release-channel"
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


def test_release_index_check_rejects_vm_obom_content_drift(tmp_path: Path) -> None:
    manifest_path = _write_release_manifest(tmp_path)
    bad_obom = b'{"bomFormat":"not-cyclonedx"}'
    obom_path = manifest_path.parent / "arm64" / "obom.cdx.json"
    obom_path.write_bytes(bad_obom)
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    manifest["assets"]["releases"]["2030.0101.1"]["arches"]["arm64"]["obom.cdx.json"] = {
        "hash": blake3(bad_obom).hexdigest(),
        "size": len(bad_obom),
    }
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    dist = tmp_path / "cache" / "target" / "release-channel"
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
    assert "VM OBOM evidence bomFormat mismatch" in result.stderr


def test_release_index_check_rejects_missing_vm_asset_attestation_evidence(
    tmp_path: Path,
) -> None:
    manifest_path = _write_release_manifest(tmp_path)
    dist = tmp_path / "cache" / "target" / "release-channel"
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


def test_release_index_check_rejects_mismatched_vm_attestation_predicate_evidence(
    tmp_path: Path,
) -> None:
    manifest_path = _write_release_manifest(tmp_path)
    dist = tmp_path / "cache" / "target" / "release-channel"
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
    vm_attestation = next(
        item
        for item in health["evidence"]["attestations"]
        if item["name"] == "github_attestations_vm_assets"
    )
    vm_attestation["predicate_url"] = "/assets/releases/2030.0101.1/missing-obom.cdx.json"
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
    assert (
        "health.json VM asset attestation predicate /assets/releases/2030.0101.1/"
        "missing-obom.cdx.json missing from VM OBOM evidence"
    ) in result.stderr


def test_release_index_check_rejects_missing_vm_attestation_predicate_evidence(
    tmp_path: Path,
) -> None:
    manifest_path = _write_release_manifest(tmp_path)
    dist = tmp_path / "cache" / "target" / "release-channel"
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
    vm_attestation = next(
        item
        for item in health["evidence"]["attestations"]
        if item["name"] == "github_attestations_vm_assets"
    )
    del vm_attestation["predicate_url"]
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
    assert "health.json VM asset attestation predicate_url missing" in result.stderr


def test_release_index_check_rejects_attestation_rail_drift(tmp_path: Path) -> None:
    manifest_path = _write_release_manifest(tmp_path)
    dist = tmp_path / "cache" / "target" / "release-channel"
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
    vm_attestation = next(
        item
        for item in health["evidence"]["attestations"]
        if item["name"] == "github_attestations_vm_assets"
    )
    vm_attestation["scope"] = "host_binaries"
    vm_attestation["workflow"] = ".github/workflows/release.yaml"
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
    assert "health.json VM asset attestation scope mismatch" in result.stderr


def test_release_index_check_rejects_missing_host_sbom_evidence(tmp_path: Path) -> None:
    manifest_path = _write_release_manifest(tmp_path)
    dist = tmp_path / "cache" / "target" / "release-channel"
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


def test_release_index_check_rejects_noncanonical_host_sbom_evidence(
    tmp_path: Path,
) -> None:
    manifest_path = _write_release_manifest(tmp_path)
    dist = tmp_path / "cache" / "target" / "release-channel"
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
    health["evidence"]["host_sboms"][0]["name"] = "not-the-canonical-sbom.json"
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
    assert "health.json host SBOM evidence name mismatch" in result.stderr


def test_release_index_check_rejects_host_binary_hash_drift(tmp_path: Path) -> None:
    manifest_path = _write_release_manifest(tmp_path)
    dist = tmp_path / "cache" / "target" / "release-channel"
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


def test_binary_release_index_records_source_on_packages_without_changing_runtime(
    tmp_path: Path,
) -> None:
    legacy_manifest = _write_release_manifest(tmp_path)
    dist = tmp_path / "cache" / "target" / "candidate-channel"
    _run_admin(
        "assets",
        "channel",
        "build",
        "--manifest",
        f"file://{legacy_manifest}",
        "--assets-dir",
        str(legacy_manifest.parent),
        "--channel",
        "stable",
        "--out-dir",
        str(dist),
        "--generated-at",
        "2030-01-01T00:00:00Z",
    )
    manifest_path = dist / "assets/stable/manifest.json"
    before = json.loads(manifest_path.read_text(encoding="utf-8"))
    artifacts = tmp_path / "release-artifacts"
    artifacts.mkdir()
    pkg = artifacts / "Capsem-1.4.2.pkg"
    deb = artifacts / "Capsem_1.4.2_arm64.deb"
    sbom = artifacts / "capsem-sbom.spdx.json"
    pkg_bytes = _write_minimal_pkg(pkg)
    deb_bytes = _write_minimal_deb(deb)
    sbom.write_bytes(b'{"spdxVersion":"SPDX-2.3","name":"capsem"}')

    result = _run_admin(
        "assets",
        "channel",
        "record-binary",
        "--manifest-path",
        str(manifest_path),
        "--version",
        "1.4.2",
        "--source-commit",
        SOURCE_COMMIT,
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
    assert report["version"] == "1.4.2"
    assert after["runtime"] == before["runtime"]
    assert "profiles" not in after
    packages = {entry["name"]: entry for entry in after["packages"]}
    assert packages[pkg.name]["source_commit"] == SOURCE_COMMIT
    assert packages[deb.name]["source_commit"] == SOURCE_COMMIT
    assert packages[pkg.name]["digest"]["sha256"] == hashlib.sha256(pkg_bytes).hexdigest()
    assert packages[deb.name]["digest"]["sha256"] == hashlib.sha256(deb_bytes).hexdigest()
    assert packages[deb.name]["binaries"]
    assert (
        packages[deb.name]["evidence"][0]["digest"]["sha256"]
        == hashlib.sha256(sbom.read_bytes()).hexdigest()
    )


def test_binary_release_index_rejects_bad_spdx_sbom(tmp_path: Path) -> None:
    manifest_path = _write_release_manifest(tmp_path)
    artifacts = tmp_path / "release-artifacts"
    artifacts.mkdir()
    pkg = artifacts / "Capsem-1.4.2.pkg"
    sbom = artifacts / "capsem-sbom.spdx.json"
    _write_minimal_pkg(pkg)
    sbom.write_bytes(b'{"spdxVersion":"SPDX-2.2","name":"capsem"}')

    result = _run_admin(
        "assets",
        "channel",
        "record-binary",
        "--manifest-path",
        str(manifest_path),
        "--version",
        "1.4.2",
        "--source-commit",
        SOURCE_COMMIT,
        "--date",
        "2030-02-03",
        "--artifact",
        str(pkg),
        "--artifact",
        str(sbom),
        "--json",
        check=False,
    )

    assert result.returncode != 0
    assert "capsem-sbom.spdx.json spdxVersion mismatch" in result.stderr


def test_binary_release_index_rejects_sbom_without_host_package(tmp_path: Path) -> None:
    manifest_path = _write_release_manifest(tmp_path)
    artifacts = tmp_path / "release-artifacts"
    artifacts.mkdir()
    sbom = artifacts / "capsem-sbom.spdx.json"
    sbom.write_bytes(b'{"spdxVersion":"SPDX-2.3","name":"capsem"}')

    result = _run_admin(
        "assets",
        "channel",
        "record-binary",
        "--manifest-path",
        str(manifest_path),
        "--version",
        "1.4.2",
        "--source-commit",
        SOURCE_COMMIT,
        "--date",
        "2030-02-03",
        "--artifact",
        str(sbom),
        "--json",
        check=False,
    )

    assert result.returncode != 0
    assert "binary release metadata must include a host package artifact" in result.stderr


def test_binary_release_index_rejects_non_package_host_artifact(tmp_path: Path) -> None:
    manifest_path = _write_release_manifest(tmp_path)
    artifacts = tmp_path / "release-artifacts"
    artifacts.mkdir()
    readme = artifacts / "release-notes.txt"
    sbom = artifacts / "capsem-sbom.spdx.json"
    readme.write_bytes(b"not an installable package")
    sbom.write_bytes(b'{"spdxVersion":"SPDX-2.3","name":"capsem"}')

    result = _run_admin(
        "assets",
        "channel",
        "record-binary",
        "--manifest-path",
        str(manifest_path),
        "--version",
        "1.4.2",
        "--source-commit",
        SOURCE_COMMIT,
        "--date",
        "2030-02-03",
        "--artifact",
        str(readme),
        "--artifact",
        str(sbom),
        "--json",
        check=False,
    )

    assert result.returncode != 0
    assert "binary release metadata must include a .pkg or .deb artifact" in result.stderr


def test_binary_release_index_rejects_empty_artifact(tmp_path: Path) -> None:
    manifest_path = _write_release_manifest(tmp_path)
    artifacts = tmp_path / "release-artifacts"
    artifacts.mkdir()
    pkg = artifacts / "Capsem-1.4.2.pkg"
    sbom = artifacts / "capsem-sbom.spdx.json"
    pkg.write_bytes(b"")
    sbom.write_bytes(b'{"spdxVersion":"SPDX-2.3","name":"capsem"}')

    result = _run_admin(
        "assets",
        "channel",
        "record-binary",
        "--manifest-path",
        str(manifest_path),
        "--version",
        "1.4.2",
        "--source-commit",
        SOURCE_COMMIT,
        "--date",
        "2030-02-03",
        "--artifact",
        str(pkg),
        "--artifact",
        str(sbom),
        "--json",
        check=False,
    )

    assert result.returncode != 0
    assert "binary release artifact is empty" in result.stderr


def test_binary_release_index_rejects_package_version_mismatch(tmp_path: Path) -> None:
    manifest_path = _write_release_manifest(tmp_path)
    artifacts = tmp_path / "release-artifacts"
    artifacts.mkdir()
    pkg = artifacts / "Capsem-1.4.0000000000.pkg"
    sbom = artifacts / "capsem-sbom.spdx.json"
    _write_minimal_pkg(pkg)
    sbom.write_bytes(b'{"spdxVersion":"SPDX-2.3","name":"capsem"}')

    result = _run_admin(
        "assets",
        "channel",
        "record-binary",
        "--manifest-path",
        str(manifest_path),
        "--version",
        "1.4.2",
        "--source-commit",
        SOURCE_COMMIT,
        "--date",
        "2030-02-03",
        "--artifact",
        str(pkg),
        "--artifact",
        str(sbom),
        "--json",
        check=False,
    )

    assert result.returncode != 0
    assert "binary release package artifact name must match version" in result.stderr


def test_binary_release_index_rejects_noncanonical_sbom_artifact(tmp_path: Path) -> None:
    manifest_path = _write_release_manifest(tmp_path)
    artifacts = tmp_path / "release-artifacts"
    artifacts.mkdir()
    pkg = artifacts / "Capsem-1.4.2.pkg"
    sbom = artifacts / "host-sbom.spdx.json"
    _write_minimal_pkg(pkg)
    sbom.write_bytes(b'{"spdxVersion":"SPDX-2.3","name":"capsem"}')

    result = _run_admin(
        "assets",
        "channel",
        "record-binary",
        "--manifest-path",
        str(manifest_path),
        "--version",
        "1.4.2",
        "--source-commit",
        SOURCE_COMMIT,
        "--date",
        "2030-02-03",
        "--artifact",
        str(pkg),
        "--artifact",
        str(sbom),
        "--json",
        check=False,
    )

    assert result.returncode != 0
    assert "capsem-sbom.spdx.json" in result.stderr


def test_binary_release_index_builds_release_site_without_rebuilding_vm_assets(
    tmp_path: Path,
) -> None:
    manifest_path = _write_release_manifest(tmp_path)
    _hydrate_asset_sha256_in_manifest(manifest_path)
    # A tag release runner must not need local VM build outputs.
    for local_asset in (tmp_path / "assets" / "arm64").iterdir():
        if local_asset.name != "software-inventory.json":
            local_asset.unlink()

    dist = tmp_path / "cache" / "target" / "release-channel"
    asset_base = "https://github.com/google/capsem/releases/download/assets-v{asset_version}"
    _run_admin(
        "assets",
        "channel",
        "build",
        "--manifest",
        f"file://{manifest_path}",
        "--assets-dir",
        str(tmp_path / "assets"),
        "--asset-source-base",
        asset_base,
        "--channel",
        "stable",
        "--out-dir",
        str(dist),
    )

    health = json.loads((dist / "health.json").read_text(encoding="utf-8"))
    assert health["current"] == {
        "binary": "1.4.1",
        "assets": "2030.0101.1",
    }
    assert health["urls"]["asset_base"] == asset_base
    rootfs_url = (
        "https://github.com/google/capsem/releases/download/assets-v2030.0101.1/arm64-rootfs.erofs"
    )
    assert any(file["url"] == rootfs_url for file in health["assets"]["files"])
    channel_manifest_text = (dist / "assets" / "stable" / "manifest.json").read_text(
        encoding="utf-8"
    )
    assert health["evidence"]["host_binary_files"]
    assert health["evidence"]["host_sboms"]
    assert health["evidence"]["attestations"]
    assert health["runtime"]["source"] == "manifest.runtime"
    assert health["updates"]["runtime"]["source"] == "manifest.runtime"
    assert '"runtime"' in channel_manifest_text
    assert '"profiles"' not in channel_manifest_text
    assert '"min_capsem_version": "1.4.0"' in channel_manifest_text
    assert "file://" not in channel_manifest_text
    assert str(tmp_path) not in channel_manifest_text
    assert rootfs_url in channel_manifest_text
    assert not (dist / "assets" / "releases").exists()
    assert not (dist / "runtime" / "releases").exists()
    _run_admin("assets", "channel", "check", "--channel", "stable", "--dist", str(dist))
