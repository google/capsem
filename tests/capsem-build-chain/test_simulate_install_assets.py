"""simulate-install asset replacement contract tests."""

from __future__ import annotations

import json
import os
import platform
import subprocess
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = PROJECT_ROOT / "scripts" / "simulate-install.sh"
BINARIES = ["capsem", "capsem-service", "capsem-process", "capsem-mcp", "capsem-gateway", "capsem-tray"]


def _host_arch() -> str:
    machine = platform.machine().lower()
    return "arm64" if machine in {"arm64", "aarch64"} else "x86_64"


def _write_fake_bins(root: Path) -> None:
    root.mkdir(parents=True)
    for name in BINARIES:
        path = root / name
        if name == "capsem":
            path.write_text("#!/bin/sh\necho 'capsem 1.0.test (build fake.1)'\n")
        else:
            path.write_text("#!/bin/sh\nexit 0\n")
        path.chmod(0o755)


def _write_assets(root: Path, initrd_prefix: str) -> tuple[str, str]:
    arch = _host_arch()
    arch_dir = root / arch
    arch_dir.mkdir(parents=True)
    (arch_dir / "vmlinuz-deadbeefdeadbeef").write_text("kernel")
    initrd_name = f"initrd-{initrd_prefix}.img"
    (arch_dir / initrd_name).write_text(f"initrd-{initrd_prefix}")
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
                            "initrd.img": {"hash": initrd_prefix + "0" * 48, "size": 6},
                            "rootfs.erofs": {"hash": "feedfacefeedface" + "0" * 48, "size": 6},
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
    return arch, initrd_name


def test_reinstall_updates_initrd_when_only_initrd_hash_changes(tmp_path: Path) -> None:
    bin_src = tmp_path / "bin"
    capsem_home = tmp_path / "home"
    assets_v1 = tmp_path / "assets-v1"
    assets_v2 = tmp_path / "assets-v2"
    _write_fake_bins(bin_src)
    arch, initrd_v1 = _write_assets(assets_v1, "1111111111111111")
    _, initrd_v2 = _write_assets(assets_v2, "2222222222222222")
    env = {
        **os.environ,
        "CAPSEM_HOME": str(capsem_home),
        "CAPSEM_RUN_DIR": str(capsem_home / "run"),
    }

    subprocess.run(["bash", str(SCRIPT), str(bin_src), str(assets_v1)], env=env, check=True)
    assert (capsem_home / "assets" / arch / initrd_v1).exists()

    subprocess.run(["bash", str(SCRIPT), str(bin_src), str(assets_v2)], env=env, check=True)

    assert (capsem_home / "assets" / "manifest.json").exists()
    assert (capsem_home / "assets" / arch / initrd_v2).exists()
    assert not (capsem_home / "assets" / arch / arch).exists()
