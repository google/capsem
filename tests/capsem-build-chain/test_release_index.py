"""Release-channel index generator contract tests."""

from __future__ import annotations

import gzip
import hashlib
import io
import json
import os
import subprocess
import sys
import tarfile
from pathlib import Path

from blake3 import blake3
from helpers.release_site import release_site_build_lock

PROJECT_ROOT = Path(__file__).resolve().parents[2]


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
    install = subprocess.run(
        ["pnpm", "install", "--frozen-lockfile"],
        cwd=PROJECT_ROOT / "release-site",
        text=True,
        capture_output=True,
        check=False,
    )
    assert install.returncode == 0, (
        f"release-site pnpm install failed\nstdout:\n{install.stdout}\nstderr:\n{install.stderr}"
    )
    # build:channel renders through the shared release-site/dist before
    # overlaying into `dist`, so it has to serialize with every other build.
    with release_site_build_lock():
        build = subprocess.run(
            ["pnpm", "run", "build:channel"],
            cwd=PROJECT_ROOT / "release-site",
            env={
                **os.environ,
                # Render from this graph, overlay into this dist.
                "CAPSEM_RELEASE_GRAPH": str(dist),
                "CAPSEM_RELEASE_CHANNEL_DIST": str(dist),
            },
            text=True,
            capture_output=True,
            check=False,
        )
    assert build.returncode == 0, (
        f"release-site Astro build failed\nstdout:\n{build.stdout}\nstderr:\n{build.stderr}"
    )


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


# A profile carries its own semver revision. The old fixture used the
# collapsed `profiles-<hash>` identifier here, which names a *set* of
# profiles at differing versions -- not a version any single profile can
# hold. capsem-admin rejects it now, correctly.
# Deliberately not the workspace version: a fixture equal to the real one
# hides whether the code under test depends on it.
PROFILE_REVISION = "1.4.0"


def _write_profile_catalog(root: Path, revision: str = PROFILE_REVISION) -> Path:
    profiles_dir = root / "config" / "profiles"
    profile_dir = profiles_dir / "code"
    profile_dir.mkdir(parents=True, exist_ok=True)
    (profile_dir / "apt-packages.txt").write_text("zstd\n", encoding="utf-8")
    (profile_dir / "python-requirements.txt").write_text("pytest==8.0.0\n", encoding="utf-8")
    (profile_dir / "python-requirements.lock").write_text(
        "pytest==8.0.0 \\\n    --hash=sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
        encoding="utf-8",
    )
    (profile_dir / "npm-packages.txt").write_text("@openai/codex@0.147.0\n", encoding="utf-8")
    (profile_dir / "npm-package-lock.json").write_text(
        json.dumps(
            {
                "name": "capsem-profile-ai-clis",
                "lockfileVersion": 3,
                "packages": {
                    "": {"dependencies": {"@openai/codex": "0.147.0"}},
                    "node_modules/@openai/codex": {
                        "version": "0.147.0",
                        "integrity": "sha512-dGVzdA==",
                    },
                },
            }
        )
        + "\n",
        encoding="utf-8",
    )
    root_payload = b"fixture profile root\n"
    (profile_dir / "root/root").mkdir(parents=True)
    (profile_dir / "root/root/.profile").write_bytes(root_payload)
    (profile_dir / "root.manifest.json").write_text(
        json.dumps(
            {
                "format": "capsem.profile-root.v1",
                "files": [
                    {
                        "path": "root/.profile",
                        "hash": f"blake3:{blake3(root_payload).hexdigest()}",
                        "size": len(root_payload),
                    }
                ],
            }
        )
        + "\n",
        encoding="utf-8",
    )
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

[assets.arch.x86_64.kernel]
name = "vmlinuz"
url = "https://release.capsem.org/assets/releases/2030.0101.1/x86_64-vmlinuz"

[assets.arch.x86_64.initrd]
name = "initrd.img"
url = "https://release.capsem.org/assets/releases/2030.0101.1/x86_64-initrd.img"

[assets.arch.x86_64.rootfs]
name = "rootfs.erofs"
url = "https://release.capsem.org/assets/releases/2030.0101.1/x86_64-rootfs.erofs"

[files.apt_packages]
path = "profiles/code/apt-packages.txt"

[files.python_requirements]
path = "profiles/code/python-requirements.txt"

[files.python_requirements_lock]
path = "profiles/code/python-requirements.lock"

[files.npm_packages]
path = "profiles/code/npm-packages.txt"

[files.npm_package_lock]
path = "profiles/code/npm-package-lock.json"

[files.root_manifest]
path = "profiles/code/root.manifest.json"
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
    assert "/profiles/stable/*\n  Cache-Control: no-cache" not in headers
    assert "/assets/releases/*\n  Cache-Control: public, max-age=31536000, immutable" in headers
    assert "/profiles/releases/*\n  Cache-Control: public, max-age=31536000, immutable" in headers
    assert "/assets/*\n  Cache-Control: no-cache" not in headers
    assert "/profiles/*\n  Cache-Control: no-cache" not in headers


def test_release_index_generator_builds_human_and_machine_outputs(tmp_path: Path) -> None:
    manifest_path = _write_release_manifest(tmp_path, include_x86_64=True)
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
    assert "Profile Catalog" not in channel_html
    assert PROFILE_REVISION in channel_html
    assert "SBOM" in channel_html
    assert "Realm Discipline" not in index_html
    assert 'href="/channels.json"' in index_html
    assert 'href="/assets/stable/manifest.json"' in channel_html
    assert "/assets/stable/manifest.json" in channel_html
    assert f"/profiles/releases/{PROFILE_REVISION}/catalog.json" not in channel_html
    assert "Capsem-1.4.1.pkg" in channel_html
    assert "capsem-1-4-1-pkg-sbom.spdx.json" in channel_html
    assert "The fastest way to ship with AI securely." not in index_html
    profile_html = (dist / "channels" / "stable" / "profiles" / "code" / "index.html").read_text(
        encoding="utf-8"
    )
    assert "Architecture arm64" in profile_html
    assert "Architecture x86_64" in profile_html
    assert "Profile Evidence" in profile_html
    assert "ABOM" in profile_html
    assert (f"/profiles/releases/stable/code/{PROFILE_REVISION}/arm64/rootfs.erofs") in profile_html
    assert (
        f"/profiles/releases/stable/code/{PROFILE_REVISION}/arm64/obom.cdx.json"
    ) in profile_html
    assert (
        f"/profiles/releases/stable/code/{PROFILE_REVISION}/x86_64/rootfs.erofs"
    ) in profile_html
    assert "APT package list" in profile_html
    assert "Python requirements" in profile_html
    assert "Python requirements lock" in profile_html
    assert "NPM package list" in profile_html
    assert "NPM package lock" in profile_html
    assert "Root manifest" in profile_html

    headers = (dist / "_headers").read_text(encoding="utf-8")
    assert "/\n  Cache-Control: no-cache, must-revalidate" in headers
    assert "/health.json\n  Cache-Control: no-cache, must-revalidate" in headers
    assert "/assets/stable/*\n  Cache-Control: no-cache, must-revalidate" in headers
    assert "/profiles/stable/*\n  Cache-Control: no-cache" not in headers
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
        "binary": "1.4.1",
        "assets": "2030.0101.1",
    }
    assert health["updates"]["binary"]["latest"] == "1.4.1"
    assert health["updates"]["assets"]["manifest"] == "/assets/stable/manifest.json"
    assert health["profiles"]["revision"] == PROFILE_REVISION
    assert health["profiles"]["source"] == "manifest.profiles"
    assert "profile_catalog" not in health["urls"]
    assert "hash" not in health["profiles"]
    assert "compatibility" not in health["profiles"]
    assert "requires_newer" not in health["profiles"]
    assert health["profiles"]["min_binary"] == "1.4.0"
    assert health["profiles"]["requires_newer_binary"] is False
    assert health["updates"]["profiles"]["latest"] == PROFILE_REVISION
    assert health["updates"]["profiles"]["current"] == PROFILE_REVISION
    assert health["updates"]["profiles"]["state"] == "current"
    assert health["updates"]["profiles"]["source"] == "manifest.profiles"
    assert "hash" not in health["updates"]["profiles"]
    assert "compatibility" not in health["updates"]["profiles"]
    assert "requires_newer" not in health["updates"]["profiles"]
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
    assert health["profiles"]["min_binary"] == "1.4.0"

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

    _run_admin("assets", "channel", "check", "--channel", "stable", "--dist", str(dist))


def test_profile_release_deploys_generated_preview_only_when_activation_ready() -> None:
    workflow = (PROJECT_ROOT / ".github/workflows/release-assets.yaml").read_text(encoding="utf-8")
    pairing = workflow.split("  test-profile-pairing:", maxsplit=1)[1].split(
        "  author-profile-release:", maxsplit=1
    )[0]
    author = workflow.split("  author-profile-release:", maxsplit=1)[1].split(
        "  publish-profile-release:", maxsplit=1
    )[0]
    publish = workflow.split("  publish-profile-release:", maxsplit=1)[1].split(
        "  deploy-channel:", maxsplit=1
    )[0]
    deploy_channel = workflow.split("  deploy-channel:", maxsplit=1)[1]

    assert "cargo run -p capsem-admin -- manifest generate assets" in author
    assert "scripts/check-profile-release-delta.py" in author
    assert "cargo run -p capsem-admin -- release" in author
    assert "scripts/build-complete-release-channel.py" in author
    assert '--channel-source "$CHANNEL=file://$PWD/assets/manifest.json"' in author
    assert '--primary-channel "$CHANNEL"' in author
    assert "--allow-mirror-missing" in author
    assert '--asset-source-base "$ASSET_BASE"' in author
    assert "releases/download/$PROFILE_IDENTITY" in author
    assert "--source-manifest target/source-channel/manifest.json" in author
    assert "--out-dir target/profile-candidate" in author
    assert "source_changed: ${{ steps.profile-delta.outputs.source_changed }}" in author
    assert "activation_needed: ${{ steps.profile-delta.outputs.activation_needed }}" in author
    assert "release_needed: ${{ steps.profile-delta.outputs.release_needed }}" in author
    assert "product_compatible: ${{ steps.author-release.outputs.product_compatible }}" in author
    assert "functional_ready: ${{ steps.author-release.outputs.functional_ready }}" in author
    assert "activation_ready: ${{ steps.author-release.outputs.activation_ready }}" in author
    assert "if: ${{ steps.profile-delta.outputs.release_needed == 'true' }}" in author

    assert "needs.author-profile-release.outputs.release_needed == 'true'" in pairing
    assert (
        pairing.count("if: ${{ needs.author-profile-release.outputs.activation_ready == 'true' }}")
        == 2
    )

    assert "needs.test-profile-pairing.result == 'success'" in publish
    assert "scripts/build-complete-release-channel.py" in publish
    assert "--out-dir target/release-channel" in publish
    assert "name: asset-channel-preview" in publish
    assert "path: target/release-channel/" in publish
    assert "if: ${{ needs.author-profile-release.outputs.activation_ready == 'true' }}" in publish

    assert (
        "if: ${{ inputs.dry_run == false && "
        "needs.publish-profile-release.outputs.release_needed == 'true' && "
        "needs.publish-profile-release.outputs.activation_ready == 'true' }}" in deploy_channel
    )
    assert "uses: ./.github/workflows/release-channel.yaml" in deploy_channel
    assert "dist_artifact: asset-channel-preview" in deploy_channel


def test_profile_release_publishes_deferred_assets_but_withholds_channel_deploy() -> None:
    workflow = (PROJECT_ROOT / ".github/workflows/release-assets.yaml").read_text(encoding="utf-8")
    reusable_fast_gate = (PROJECT_ROOT / ".github/workflows/fast-gate.yaml").read_text(
        encoding="utf-8"
    )
    pairing = workflow.split("  test-profile-pairing:", maxsplit=1)[1].split(
        "  author-profile-release:", maxsplit=1
    )[0]
    author = workflow.split("  author-profile-release:", maxsplit=1)[1].split(
        "  publish-profile-release:", maxsplit=1
    )[0]
    publish = workflow.split("  publish-profile-release:", maxsplit=1)[1].split(
        "  deploy-channel:", maxsplit=1
    )[0]
    deploy_channel = workflow.split("  deploy-channel:", maxsplit=1)[1]

    assert "scripts/check-profile-release-delta.py" in author
    assert "cargo run -p capsem-admin -- release" in author
    assert "Run shared artifact module" in pairing
    assert "Run complete shared fast module" in reusable_fast_gate
    assert "run: just _test-fast" in reusable_fast_gate
    assert "Run shared release contracts" not in pairing
    assert (
        "if:"
        not in pairing.split("- name: Run shared artifact module", maxsplit=1)[1].split(
            "- name: Record deferred profile staging boundary", maxsplit=1
        )[0]
    )
    assert "activation-ready profile cannot defer complete pairing gates" in pairing
    assert "complete functional release binary cohort" in pairing
    assert "needs.author-profile-release.outputs.activation_ready != 'true'" in pairing
    assert "Publish immutable GitHub profile release" in publish
    immutable_release = publish.split(
        "- name: Publish immutable GitHub profile release", maxsplit=1
    )[1].split("- uses: actions/upload-artifact@", maxsplit=1)[0]
    assert "outputs.activation_ready" not in immutable_release
    assert "needs.publish-profile-release.outputs.activation_ready == 'true'" in deploy_channel


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
    assert (
        "health.json profile update latest target does not match manifest profiles" in result.stderr
    )


def test_release_index_check_rejects_stale_human_index_state(tmp_path: Path) -> None:
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


def test_release_index_check_rejects_profile_catalog_content_drift(tmp_path: Path) -> None:
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

    manifest_path = dist / "assets" / "stable" / "manifest.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    profile = next(iter(manifest["profiles"].values()))
    profile["revision"] = "profiles-stale"
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


def test_release_index_check_rejects_mismatched_vm_attestation_predicate_evidence(
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


def test_release_index_check_rejects_noncanonical_host_sbom_evidence(
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
    assert report["min_assets"] == before["assets"]["current"]
    assert after["assets"] == before["assets"]
    assert after["binaries"]["current"] == "1.4.2"
    release = after["binaries"]["releases"]["1.4.2"]
    assert release["min_assets"] == "2030.0101.1"
    assert release["version"] == "1.4.2"
    assert release["date"] == "2030-02-03"
    files = {entry["name"]: entry for entry in release["files"]}
    assert files[pkg.name]["sha256"] == hashlib.sha256(pkg_bytes).hexdigest()
    assert files[deb.name]["sha256"] == hashlib.sha256(deb_bytes).hexdigest()
    assert files[deb.name]["binaries"]
    assert files[sbom.name]["sha256"] == hashlib.sha256(sbom.read_bytes()).hexdigest()


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


def test_binary_release_profile_catalog_index_builds_release_site_without_rebuilding_vm_assets(
    tmp_path: Path,
) -> None:
    manifest_path = _write_release_manifest(tmp_path)
    _hydrate_asset_sha256_in_manifest(manifest_path)
    profiles_dir = _write_profile_catalog(tmp_path)
    # A tag release runner must not need local VM build outputs.
    for local_asset in (tmp_path / "assets" / "arm64").iterdir():
        if local_asset.name != "software-inventory.json":
            local_asset.unlink()

    dist = tmp_path / "target" / "release-channel"
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
        "--profiles-dir",
        str(profiles_dir),
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
    assert health["profiles"]["source"] == "manifest.profiles"
    assert health["updates"]["profiles"]["source"] == "manifest.profiles"
    assert '"profiles"' in channel_manifest_text
    assert '"min_capsem_version": "1.4.0"' in channel_manifest_text
    assert "file://" not in channel_manifest_text
    assert str(tmp_path) not in channel_manifest_text
    assert rootfs_url in channel_manifest_text
    assert not (dist / "assets" / "releases").exists()
    _run_admin("assets", "channel", "check", "--channel", "stable", "--dist", str(dist))
