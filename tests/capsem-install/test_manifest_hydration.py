"""Manifest hydration contract for installed updates.

Packages install a manifest and provenance; VM payloads are hydrated later
through that manifest. This test proves a local ``file://`` manifest source
uses the same hash-named asset layout as remote downloads without bundling VM
asset blobs into the package.
"""

from __future__ import annotations

import json
import os
import platform
import subprocess
from pathlib import Path

import pytest

from .conftest import INSTALL_DIR
from .test_asset_download import _blake3, _make_manifest


def _arch() -> str:
    machine = platform.machine().lower()
    return "arm64" if machine in ("arm64", "aarch64") else "x86_64"


def _hash_filename(logical_name: str, digest: str) -> str:
    prefix = digest[:16]
    if "." in logical_name:
        stem, ext = logical_name.split(".", 1)
        return f"{stem}-{prefix}.{ext}"
    return f"{logical_name}-{prefix}"


def test_update_assets_hydrates_from_manifest_origin_file_url(
    tmp_path: Path,
    installed_layout,
) -> None:
    arch = _arch()
    source_assets = tmp_path / "source-assets"
    (source_assets / arch).mkdir(parents=True)

    files = {
        "vmlinuz": b"manifest-hydration-kernel",
        "initrd.img": b"manifest-hydration-initrd",
        "rootfs.erofs": b"manifest-hydration-rootfs",
    }
    for name, data in files.items():
        (source_assets / arch / name).write_bytes(data)
    manifest = _make_manifest(arch, files)
    manifest_path = source_assets / "manifest.json"
    manifest_path.write_text(json.dumps(manifest), encoding="utf-8")

    capsem_home = tmp_path / ".capsem"
    installed_assets = capsem_home / "assets"
    installed_assets.mkdir(parents=True)
    (installed_assets / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
    (installed_assets / "manifest-origin.json").write_text(
        json.dumps(
            {
                "schema": "capsem.manifest_origin.v1",
                "origin": "package",
                "source": manifest_path.as_uri(),
                "packaged_at": "2026-06-16T00:00:00Z",
            },
            sort_keys=True,
        )
        + "\n",
        encoding="utf-8",
    )

    result = subprocess.run(
        [str(INSTALL_DIR / "capsem"), "update", "--assets"],
        capture_output=True,
        text=True,
        timeout=30,
        env={
            **os.environ,
            "CAPSEM_HOME": str(capsem_home),
            "CAPSEM_RUN_DIR": str(capsem_home / "run"),
        },
    )
    assert result.returncode == 0, (
        f"capsem update --assets failed\nstdout={result.stdout}\nstderr={result.stderr}"
    )
    assert f"Using local asset source {source_assets}" in result.stdout

    for logical_name, data in files.items():
        digest = _blake3(data)
        target = installed_assets / arch / _hash_filename(logical_name, digest)
        assert target.is_file(), f"missing hydrated asset {target}"
        assert target.read_bytes() == data
        assert (target.stat().st_mode & 0o777) == 0o444


def test_update_assets_manifest_override_hydrates_from_file_url(
    tmp_path: Path,
    installed_layout,
) -> None:
    arch = _arch()
    source_assets = tmp_path / "source-assets"
    (source_assets / arch).mkdir(parents=True)

    files = {
        "vmlinuz": b"manifest-override-kernel",
        "initrd.img": b"manifest-override-initrd",
        "rootfs.erofs": b"manifest-override-rootfs",
    }
    for name, data in files.items():
        (source_assets / arch / name).write_bytes(data)
    manifest = _make_manifest(arch, files)
    manifest_path = source_assets / "manifest.json"
    manifest_path.write_text(json.dumps(manifest), encoding="utf-8")

    capsem_home = tmp_path / ".capsem"
    installed_assets = capsem_home / "assets"
    result = subprocess.run(
        [
            str(INSTALL_DIR / "capsem"),
            "update",
            "--assets",
            "--manifest",
            manifest_path.as_uri(),
        ],
        capture_output=True,
        text=True,
        timeout=30,
        env={
            **os.environ,
            "CAPSEM_HOME": str(capsem_home),
            "CAPSEM_RUN_DIR": str(capsem_home / "run"),
        },
    )

    assert result.returncode == 0, (
        "capsem update --assets --manifest file://... failed\n"
        f"stdout={result.stdout}\nstderr={result.stderr}"
    )
    assert f"Installed asset manifest from {manifest_path.as_uri()}" in result.stdout
    assert f"Using local asset source {source_assets}" in result.stdout

    installed_manifest = json.loads((installed_assets / "manifest.json").read_text())
    assert installed_manifest == manifest
    origin = json.loads((installed_assets / "manifest-origin.json").read_text())
    assert origin["schema"] == "capsem.manifest_origin.v1"
    assert origin["origin"] == "update"
    assert origin["source"] == manifest_path.as_uri()

    for logical_name, data in files.items():
        digest = _blake3(data)
        target = installed_assets / arch / _hash_filename(logical_name, digest)
        assert target.is_file(), f"missing hydrated asset {target}"
        assert target.read_bytes() == data
        assert (target.stat().st_mode & 0o777) == 0o444


def test_update_assets_rejects_bare_manifest_origin_path_and_lists_allowed_url_schemes(
    tmp_path: Path,
    installed_layout,
) -> None:
    arch = _arch()
    files = {
        "vmlinuz": b"k",
        "initrd.img": b"i",
        "rootfs.erofs": b"r",
    }
    source_assets = tmp_path / "source-assets"
    source_assets.mkdir()
    manifest_path = source_assets / "manifest.json"
    manifest_path.write_text(json.dumps(_make_manifest(arch, files)), encoding="utf-8")

    capsem_home = tmp_path / ".capsem"
    installed_assets = capsem_home / "assets"
    installed_assets.mkdir(parents=True)
    (installed_assets / "manifest.json").write_text(
        json.dumps(_make_manifest(arch, files)),
        encoding="utf-8",
    )
    (installed_assets / "manifest-origin.json").write_text(
        json.dumps(
            {
                "schema": "capsem.manifest_origin.v1",
                "origin": "package",
                "source": str(manifest_path),
                "packaged_at": "2026-06-16T00:00:00Z",
            },
            sort_keys=True,
        )
        + "\n",
        encoding="utf-8",
    )

    result = subprocess.run(
        [str(INSTALL_DIR / "capsem"), "update", "--assets"],
        capture_output=True,
        text=True,
        timeout=30,
        env={
            **os.environ,
            "CAPSEM_HOME": str(capsem_home),
            "CAPSEM_RUN_DIR": str(capsem_home / "run"),
        },
    )

    assert result.returncode != 0
    err = result.stdout + result.stderr
    assert "asset manifest origin source must be a URL" in err, err
    assert "https://..." in err, err
    assert "http://..." in err, err
    assert "file:///absolute/path" in err, err


@pytest.mark.parametrize("flag", ["--manifest", "--corp"])
@pytest.mark.parametrize("source_kind", ["absolute_path", "relative_path"])
def test_update_url_overrides_reject_bare_paths_and_list_allowed_url_schemes(
    tmp_path: Path,
    installed_layout,
    flag: str,
    source_kind: str,
) -> None:
    manifest_path = tmp_path / "assets" / "manifest.json"
    manifest_path.parent.mkdir()
    manifest_path.write_text(
        json.dumps(
            _make_manifest(
                _arch(),
                {
                    "vmlinuz": b"k",
                    "initrd.img": b"i",
                    "rootfs.erofs": b"r",
                },
            )
        ),
        encoding="utf-8",
    )
    capsem_home = tmp_path / ".capsem"
    source = (
        str(manifest_path)
        if source_kind == "absolute_path"
        else "assets/stable/manifest.json"
    )

    command = [str(INSTALL_DIR / "capsem"), "update"]
    if flag == "--manifest":
        command.append("--assets")
    command.extend([flag, source])

    result = subprocess.run(
        command,
        capture_output=True,
        text=True,
        timeout=30,
        env={
            **os.environ,
            "CAPSEM_HOME": str(capsem_home),
            "CAPSEM_RUN_DIR": str(capsem_home / "run"),
        },
    )

    assert result.returncode != 0
    err = result.stdout + result.stderr
    assert f"{flag} must be a URL" in err, err
    assert "https://..." in err, err
    assert "http://..." in err, err
    assert "file:///absolute/path" in err, err


@pytest.mark.parametrize("flag", ["--manifest", "--corp"])
@pytest.mark.parametrize(
    ("source", "expected"),
    [
        (
            "file:assets/stable/manifest.json",
            "file URL must start with file://",
        ),
        (
            "https:release.capsem.org/assets/stable/manifest.json",
            "must use https://, http://, or file:// URLs",
        ),
    ],
)
def test_update_url_overrides_reject_url_shorthand_paths(
    tmp_path: Path,
    installed_layout,
    flag: str,
    source: str,
    expected: str,
) -> None:
    capsem_home = tmp_path / ".capsem"

    command = [str(INSTALL_DIR / "capsem"), "update"]
    if flag == "--manifest":
        command.append("--assets")
    command.extend([flag, source])

    result = subprocess.run(
        command,
        capture_output=True,
        text=True,
        timeout=30,
        env={
            **os.environ,
            "CAPSEM_HOME": str(capsem_home),
            "CAPSEM_RUN_DIR": str(capsem_home / "run"),
        },
    )

    assert result.returncode != 0
    err = result.stdout + result.stderr
    assert expected in err, err


@pytest.mark.parametrize("flag", ["--manifest", "--corp"])
def test_update_url_overrides_reject_unsupported_url_schemes(
    tmp_path: Path,
    installed_layout,
    flag: str,
) -> None:
    capsem_home = tmp_path / ".capsem"
    source = "ftp://example.invalid/capsem/manifest.json"

    command = [str(INSTALL_DIR / "capsem"), "update"]
    if flag == "--manifest":
        command.append("--assets")
    command.extend([flag, source])

    result = subprocess.run(
        command,
        capture_output=True,
        text=True,
        timeout=30,
        env={
            **os.environ,
            "CAPSEM_HOME": str(capsem_home),
            "CAPSEM_RUN_DIR": str(capsem_home / "run"),
        },
    )

    assert result.returncode != 0
    err = result.stdout + result.stderr
    assert f"unsupported {flag} URL scheme ftp" in err, err
    assert "https://" in err, err
    assert "http://" in err, err
    assert "file://" in err, err


def test_update_assets_rejects_corp_policy_source(
    tmp_path: Path,
    installed_layout,
) -> None:
    capsem_home = tmp_path / ".capsem"

    result = subprocess.run(
        [
            str(INSTALL_DIR / "capsem"),
            "update",
            "--assets",
            "--corp",
            "https://corp.example/capsem/corp.toml",
        ],
        capture_output=True,
        text=True,
        timeout=30,
        env={
            **os.environ,
            "CAPSEM_HOME": str(capsem_home),
            "CAPSEM_RUN_DIR": str(capsem_home / "run"),
        },
    )

    assert result.returncode != 0
    err = result.stdout + result.stderr
    assert "cannot be used with" in err, err
    assert "--assets" in err, err
