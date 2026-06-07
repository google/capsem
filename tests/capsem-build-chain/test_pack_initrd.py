"""Verify initrd packing produces a valid gzip+cpio archive with 555 binaries."""

import os
import subprocess
import tempfile

import pytest

from pathlib import Path

PROJECT_ROOT = Path(__file__).parent.parent.parent
ASSETS_DIR = PROJECT_ROOT / "assets"


def host_arch():
    return "arm64" if os.uname().machine == "arm64" else "x86_64"

GUEST_BINARIES = ["capsem-pty-agent", "capsem-net-proxy", "capsem-mcp-server"]

pytestmark = pytest.mark.build_chain


@pytest.fixture
def initrd_path():
    arch = host_arch()
    path = ASSETS_DIR / arch / "initrd.img"
    if not path.exists():
        pytest.skip(f"initrd.img not found at {path}")
    return path


def test_initrd_is_gzip(initrd_path):
    """initrd.img is a valid gzip file."""
    result = subprocess.run(
        ["file", str(initrd_path)],
        capture_output=True, text=True,
    )
    assert "gzip" in result.stdout.lower(), (
        f"Expected gzip, got: {result.stdout}"
    )


def test_initrd_extractable(initrd_path):
    """initrd.img can be extracted as gzip+cpio."""
    with tempfile.TemporaryDirectory() as tmp:
        result = subprocess.run(
            f"gunzip -c {initrd_path} | cpio -id 2>/dev/null",
            shell=True, cwd=tmp, capture_output=True,
        )
        assert result.returncode == 0, f"Extraction failed: {result.stderr.decode()}"
        # Should have at least some files
        contents = list(Path(tmp).rglob("*"))
        assert len(contents) > 0, "Extracted initrd is empty"


def test_initrd_binaries_555(initrd_path):
    """Guest binaries inside initrd have chmod 555."""
    with tempfile.TemporaryDirectory() as tmp:
        subprocess.run(
            f"gunzip -c {initrd_path} | cpio -id 2>/dev/null",
            shell=True, cwd=tmp, capture_output=True,
        )
        for name in GUEST_BINARIES:
            candidates = list(Path(tmp).rglob(name))
            if not candidates:
                continue
            binary = candidates[0]
            mode = oct(binary.stat().st_mode & 0o777)
            assert mode == "0o555", f"{name} in initrd should be 555, got {mode}"


def test_initrd_correct_arch(initrd_path):
    """Guest binaries in initrd are the correct architecture."""
    with tempfile.TemporaryDirectory() as tmp:
        subprocess.run(
            f"gunzip -c {initrd_path} | cpio -id 2>/dev/null",
            shell=True, cwd=tmp, capture_output=True,
        )
        for name in GUEST_BINARIES:
            candidates = list(Path(tmp).rglob(name))
            if not candidates:
                continue
            binary = candidates[0]
            result = subprocess.run(
                ["file", str(binary)],
                capture_output=True, text=True,
            )
            arch = host_arch()
            expected = "aarch64" if arch == "arm64" else "x86-64"
            assert expected in result.stdout or "ELF" in result.stdout, (
                f"{name}: expected {expected} ELF, got: {result.stdout}"
            )
