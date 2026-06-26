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

