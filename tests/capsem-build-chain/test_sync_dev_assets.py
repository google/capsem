"""Contract tests for local dev asset sync used by `just install`."""

from __future__ import annotations

import json
import platform
import subprocess
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = PROJECT_ROOT / "scripts" / "sync-dev-assets.sh"


def _host_arch() -> str:
    machine = platform.machine().lower()
    return "arm64" if machine in {"arm64", "aarch64"} else "x86_64"


def _write_assets(root: Path, *, literal: bool = False) -> str:
    arch = _host_arch()
    arch_dir = root / arch
    arch_dir.mkdir(parents=True)
    if literal:
        (arch_dir / "vmlinuz").write_text("kernel")
        (arch_dir / "initrd.img").write_text("initrd")
        (arch_dir / "rootfs.erofs").write_text("rootfs")
    else:
        (arch_dir / "vmlinuz-deadbeefdeadbeef").write_text("kernel")
        (arch_dir / "initrd-cafebabecafebabe.img").write_text("initrd")
        (arch_dir / "rootfs-feedfacefeedface.erofs").write_text("rootfs")
    (arch_dir / arch).mkdir()
    manifest = {
        "format": 2,
        "refresh_policy": "24h",
        "assets": {
            "current": "2030.0101.1",
            "releases": {
                "2030.0101.1": {
                    "arches": {
                        arch: {
                            "vmlinuz": {"hash": "deadbeefdeadbeef" + "0" * 48, "size": 6},
                            "initrd.img": {
                                "hash": "cafebabecafebabe" + "0" * 48,
                                "size": 6,
                            },
                            "rootfs.erofs": {
                                "hash": "feedfacefeedface" + "0" * 48,
                                "size": 6,
                            },
                        }
                    }
                }
            },
        },
        "binaries": {
            "current": "1.0.0",
            "releases": {"1.0.0": {"min_assets": "2030.0101.1"}},
        },
    }
    (root / "manifest.json").write_text(json.dumps(manifest))
    return arch


def test_sync_dev_assets_replaces_stale_assets_symlink(tmp_path: Path) -> None:
    src = tmp_path / "src-assets"
    stale_target = tmp_path / "old-worktree-assets"
    dst = tmp_path / "installed-assets"
    arch = _write_assets(src)
    stale_target.mkdir()
    dst.symlink_to(stale_target, target_is_directory=True)

    result = subprocess.run(
        ["bash", str(SCRIPT), str(src), str(dst)],
        cwd=PROJECT_ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
    )

    assert "Removing asset symlink" in result.stdout
    assert not dst.is_symlink()
    assert (dst / "manifest.json").exists()
    assert (dst / arch / "rootfs-feedfacefeedface.erofs").exists()
    assert not (dst / arch / arch).exists()
    assert not (stale_target / "manifest.json").exists()


def test_sync_dev_assets_replaces_symlink_back_to_source(tmp_path: Path) -> None:
    src = tmp_path / "src-assets"
    dst = tmp_path / "installed-assets"
    arch = _write_assets(src)
    dst.symlink_to(src, target_is_directory=True)

    result = subprocess.run(
        ["bash", str(SCRIPT), str(src), str(dst)],
        cwd=PROJECT_ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
    )

    assert "Removing asset symlink" in result.stdout
    assert not dst.is_symlink()
    assert (dst / "manifest.json").exists()
    assert (dst / arch / "rootfs-feedfacefeedface.erofs").exists()
    assert src.is_dir()
    assert (src / "manifest.json").exists()


def test_sync_dev_assets_materializes_hash_names_from_literal_build_output(
    tmp_path: Path,
) -> None:
    src = tmp_path / "src-assets"
    dst = tmp_path / "installed-assets"
    arch = _write_assets(src, literal=True)

    subprocess.run(
        ["bash", str(SCRIPT), str(src), str(dst)],
        cwd=PROJECT_ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
    )

    assert (dst / "manifest.json").exists()
    assert (dst / arch / "vmlinuz-deadbeefdeadbeef").exists()
    assert (dst / arch / "initrd-cafebabecafebabe.img").exists()
    assert (dst / arch / "rootfs-feedfacefeedface.erofs").exists()
    assert not (dst / arch / "vmlinuz").exists()
    assert not (dst / arch / "initrd.img").exists()
    assert not (dst / arch / "rootfs.erofs").exists()


def test_sync_dev_assets_writes_local_manifest_metadata(tmp_path: Path) -> None:
    src = tmp_path / "src-assets"
    dst = tmp_path / "installed-assets"
    _write_assets(src, literal=True)

    subprocess.run(
        ["bash", str(SCRIPT), str(src), str(dst)],
        cwd=PROJECT_ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
    )

    origin_path = dst / "manifest-metadata.json"
    assert origin_path.is_file()
    origin = json.loads(origin_path.read_text())
    assert origin["schema"] == "capsem.manifest_metadata.v1"
    assert origin["origin"] == "local-dev-sync"
    assert origin["manifest_url"] == (src / "manifest.json").resolve().as_uri()


def test_sync_dev_assets_removes_stale_hash_names(tmp_path: Path) -> None:
    src = tmp_path / "src-assets"
    dst = tmp_path / "installed-assets"
    arch = _write_assets(src, literal=True)
    stale_dir = dst / arch
    stale_dir.mkdir(parents=True)
    (stale_dir / "initrd-1111111111111111.img").write_text("old-initrd")
    (stale_dir / "rootfs-2222222222222222.erofs").write_text("old-rootfs")
    (stale_dir / "keep-me.txt").write_text("not a boot asset alias")

    subprocess.run(
        ["bash", str(SCRIPT), str(src), str(dst)],
        cwd=PROJECT_ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
    )

    assert (dst / arch / "initrd-cafebabecafebabe.img").exists()
    assert not (dst / arch / "initrd-1111111111111111.img").exists()
    assert not (dst / arch / "rootfs-2222222222222222.erofs").exists()
    assert (dst / arch / "keep-me.txt").exists()
