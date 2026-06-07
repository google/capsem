"""Verify all daemon binaries are codesigned. FAIL, not skip."""

import os
import subprocess

import pytest

from pathlib import Path

PROJECT_ROOT = Path(__file__).parent.parent.parent
TARGET_DIR = PROJECT_ROOT / "target" / "debug"
IS_MACOS = os.uname().sysname == "Darwin"
SIGNED_BINARIES = {
    "capsem-process": TARGET_DIR / "capsem-process",
    "capsem-service": TARGET_DIR / "capsem-service",
    "capsem": TARGET_DIR / "capsem",
    "capsem-mcp": TARGET_DIR / "capsem-mcp",
}

pytestmark = pytest.mark.codesign


@pytest.mark.skipif(not IS_MACOS, reason="macOS only")
class TestAllBinariesSigned:

    def test_process_signed(self):
        """capsem-process is signed with valid signature."""
        binary = SIGNED_BINARIES["capsem-process"]
        assert binary.exists(), f"capsem-process not built at {binary}"
        result = subprocess.run(
            ["codesign", "--verify", "--verbose", str(binary)],
            capture_output=True, text=True,
        )
        assert result.returncode == 0, (
            f"capsem-process not signed: {result.stderr}"
        )

    def test_service_signed(self):
        """capsem-service is signed with valid signature."""
        binary = SIGNED_BINARIES["capsem-service"]
        assert binary.exists(), f"capsem-service not built at {binary}"
        result = subprocess.run(
            ["codesign", "--verify", "--verbose", str(binary)],
            capture_output=True, text=True,
        )
        assert result.returncode == 0, (
            f"capsem-service not signed: {result.stderr}"
        )

    def test_cli_signed(self):
        """capsem CLI is signed with valid signature."""
        binary = SIGNED_BINARIES["capsem"]
        assert binary.exists(), f"capsem not built at {binary}"
        result = subprocess.run(
            ["codesign", "--verify", "--verbose", str(binary)],
            capture_output=True, text=True,
        )
        assert result.returncode == 0, (
            f"capsem not signed: {result.stderr}"
        )

    def test_mcp_signed(self):
        """capsem-mcp is signed with valid signature."""
        binary = SIGNED_BINARIES["capsem-mcp"]
        assert binary.exists(), f"capsem-mcp not built at {binary}"
        result = subprocess.run(
            ["codesign", "--verify", "--verbose", str(binary)],
            capture_output=True, text=True,
        )
        assert result.returncode == 0, (
            f"capsem-mcp not signed: {result.stderr}"
        )
