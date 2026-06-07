"""Tests for the reusable `.deb` payload verifier."""

from __future__ import annotations

import importlib.util
import subprocess
import sys
import tarfile
from io import BytesIO
from pathlib import Path

import zstandard


REPO_ROOT = Path(__file__).parent.parent
VERIFY_SCRIPT = REPO_ROOT / "scripts" / "verify_deb_payload.py"

spec = importlib.util.spec_from_file_location("verify_deb_payload", VERIFY_SCRIPT)
assert spec and spec.loader
verify_deb_payload = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = verify_deb_payload
spec.loader.exec_module(verify_deb_payload)


REQUIRED_PAYLOADS = verify_deb_payload.REQUIRED_PAYLOADS


def _tar_gz(files: dict[str, bytes]) -> bytes:
    out = BytesIO()
    with tarfile.open(fileobj=out, mode="w:gz") as tar:
        for name, data in files.items():
            info = tarfile.TarInfo(name)
            info.size = len(data)
            info.mode = 0o755 if name.startswith("./usr/bin/") else 0o644
            tar.addfile(info, BytesIO(data))
    return out.getvalue()


def _tar_zst_without_content_size(files: dict[str, bytes]) -> bytes:
    out = BytesIO()
    with tarfile.open(fileobj=out, mode="w:") as tar:
        for name, data in files.items():
            info = tarfile.TarInfo(name)
            info.size = len(data)
            info.mode = 0o755 if name.startswith("./usr/bin/") else 0o644
            tar.addfile(info, BytesIO(data))
    return zstandard.ZstdCompressor(write_content_size=False).compress(out.getvalue())


def _ar_member(name: str, data: bytes) -> bytes:
    encoded_name = (name + "/").encode()
    header = (
        encoded_name.ljust(16, b" ")
        + b"0".ljust(12, b" ")
        + b"0".ljust(6, b" ")
        + b"0".ljust(6, b" ")
        + b"100644".ljust(8, b" ")
        + str(len(data)).encode().ljust(10, b" ")
        + b"`\n"
    )
    padding = b"\n" if len(data) % 2 else b""
    return header + data + padding


def _write_deb(
    path: Path,
    *,
    version: str = "1.1.0",
    architecture: str = "amd64",
    omit: str | None = None,
    data_member: str = "data.tar.gz",
) -> None:
    control = _tar_gz({
        "./control": (
            f"Package: capsem\nVersion: {version}\nArchitecture: {architecture}\n"
            "Maintainer: test\nDescription: Capsem\n"
        ).encode(),
    })
    payload_files = {
        "./" + name: b"payload\n"
        for name in REQUIRED_PAYLOADS
        if name != omit
    }
    payload_files["./usr/share/capsem/assets/manifest.json"] = b'{"format":2}\n'
    payload_files["./usr/share/capsem/assets/manifest.json.minisig"] = b"sig\n"
    data = (
        _tar_zst_without_content_size(payload_files)
        if data_member == "data.tar.zst"
        else _tar_gz(payload_files)
    )
    path.write_bytes(
        b"!<arch>\n"
        + _ar_member("debian-binary", b"2.0\n")
        + _ar_member("control.tar.gz", control)
        + _ar_member(data_member, data)
    )


def test_verify_deb_accepts_required_payloads(tmp_path):
    deb = tmp_path / "Capsem_1.1.0_amd64.deb"
    _write_deb(deb)

    verify_deb_payload.verify_deb(
        deb,
        expected_version="1.1.0",
        expected_architecture="amd64",
        minisign_pubkey=None,
    )


def test_verify_deb_accepts_zstd_payload_without_content_size(tmp_path):
    deb = tmp_path / "Capsem_1.1.0_amd64.deb"
    _write_deb(deb, data_member="data.tar.zst")

    verify_deb_payload.verify_deb(
        deb,
        expected_version="1.1.0",
        expected_architecture="amd64",
        minisign_pubkey=None,
    )


def test_verify_deb_rejects_missing_required_payload(tmp_path):
    deb = tmp_path / "Capsem_1.1.0_amd64.deb"
    _write_deb(deb, omit="usr/bin/capsem-admin")

    try:
        verify_deb_payload.verify_deb(
            deb,
            expected_version="1.1.0",
            expected_architecture="amd64",
            minisign_pubkey=None,
        )
    except verify_deb_payload.VerificationError as exc:
        assert "usr/bin/capsem-admin" in str(exc)
    else:
        raise AssertionError("missing payload was accepted")


def test_cli_checks_control_metadata(tmp_path):
    deb = tmp_path / "Capsem_1.1.0_arm64.deb"
    _write_deb(deb, architecture="arm64")

    result = subprocess.run(
        [
            "python3",
            str(VERIFY_SCRIPT),
            str(deb),
            "--version",
            "1.1.0",
            "--architecture",
            "amd64",
        ],
        capture_output=True,
        text=True,
        timeout=10,
    )

    assert result.returncode != 0
    assert "expected Architecture: amd64" in result.stderr
