"""Regression coverage for the packaged-host SBOM generator."""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

import pytest


PROJECT_ROOT = Path(__file__).resolve().parent.parent


def _module():
    path = PROJECT_ROOT / "scripts" / "generate-host-binary-sbom.py"
    spec = importlib.util.spec_from_file_location("generate_host_binary_sbom", path)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def _write_deb_member(path: Path, name: str, payload: bytes) -> None:
    encoded_name = f"{name}/".encode("ascii").ljust(16)
    header = (
        encoded_name
        + b"0".ljust(12)
        + b"0".ljust(6)
        + b"0".ljust(6)
        + b"100644".ljust(8)
        + str(len(payload)).encode("ascii").ljust(10)
        + b"`\n"
    )
    path.write_bytes(b"!<arch>\n" + header + payload + (b"\n" if len(payload) % 2 else b""))


def test_zstd_deb_without_decoder_fails_before_invoking_tar(tmp_path, monkeypatch) -> None:
    artifact = tmp_path / "capsem.deb"
    _write_deb_member(artifact, "data.tar.zst", b"not-needed-for-preflight")
    monkeypatch.setenv("PATH", "")

    with pytest.raises(SystemExit, match=r"zstd.*required.*data\.tar\.zst"):
        _module().deb_entries(artifact)
