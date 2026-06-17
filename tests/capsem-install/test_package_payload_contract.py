"""Release package payload contract.

The package may carry host binaries, service metadata, UI assets, profile
configuration, and the manifest/provenance ledger. It must not carry VM asset
blobs such as rootfs, initrd, kernels, EROFS, QCOW, or squashfs images.
"""

from __future__ import annotations

import importlib.util
import shutil
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]


def _load_test_module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


@pytest.mark.skipif(
    shutil.which("pkgutil") is None
    or shutil.which("pkgbuild") is None
    or shutil.which("productbuild") is None,
    reason="macOS package tools not available",
)
def test_macos_pkg_payload_is_closed_and_manifest_only(tmp_path: Path) -> None:
    build_pkg = _load_test_module(
        "capsem_test_build_pkg_payload_contract",
        REPO_ROOT / "tests" / "test_build_pkg.py",
    )
    build_pkg.test_macos_pkg_payload_is_closed_and_manifest_only_for_assets(tmp_path)


@pytest.mark.skipif(
    shutil.which("dpkg-deb") is None,
    reason="dpkg-deb not on PATH (install on macOS via `brew install dpkg`)",
)
def test_deb_payload_is_closed_and_manifest_only(tmp_path: Path) -> None:
    repack_deb = _load_test_module(
        "capsem_test_repack_deb_payload_contract",
        REPO_ROOT / "tests" / "test_repack_deb.py",
    )
    repack_deb.test_repacked_deb_payload_is_closed_and_manifest_only_for_assets(
        tmp_path
    )
