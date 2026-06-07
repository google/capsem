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


def _write_assets(root: Path) -> str:
    arch = _host_arch()
    arch_dir = root / arch
    arch_dir.mkdir(parents=True)
    (arch_dir / "vmlinuz-deadbeefdeadbeef").write_text("kernel")
    (arch_dir / "initrd-cafebabecafebabe.img").write_text("initrd")
    (arch_dir / "rootfs-feedfacefeedface.erofs").write_text("rootfs")
    (arch_dir / arch).mkdir()
    manifest = {
        "format": 2,
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

    assert "Removing stale asset symlink" in result.stdout
    assert not dst.is_symlink()
    assert (dst / "manifest.json").exists()
    assert (dst / arch / "rootfs-feedfacefeedface.erofs").exists()
    assert not (dst / arch / arch).exists()
    assert not (stale_target / "manifest.json").exists()
