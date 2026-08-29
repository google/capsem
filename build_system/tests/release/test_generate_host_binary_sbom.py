"""Regression coverage for the packaged-host SBOM generator."""

from __future__ import annotations

from pathlib import Path

import pytest
from capsem_builder.release.tools import generate_host_binary_sbom as HOST_SBOM


def _module():
    return HOST_SBOM


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
