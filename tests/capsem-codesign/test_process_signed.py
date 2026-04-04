"""Verify capsem-process has the virtualization entitlement. FAIL, not skip."""

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
class TestProcessEntitlement:

    def test_has_virtualization_entitlement(self):
        """capsem-process must have com.apple.security.virtualization."""
        binary = SIGNED_BINARIES["capsem-process"]
        assert binary.exists(), f"capsem-process not built at {binary}"

        result = subprocess.run(
            ["codesign", "-d", "--entitlements", "-", "--xml", str(binary)],
            capture_output=True, text=True,
        )
        combined = result.stdout + result.stderr
        assert "com.apple.security.virtualization" in combined, (
            "capsem-process missing virtualization entitlement"
        )

    def test_codesign_verify_succeeds(self):
        """codesign --verify returns 0 for capsem-process."""
        binary = SIGNED_BINARIES["capsem-process"]
        assert binary.exists(), f"capsem-process not built at {binary}"

        result = subprocess.run(
            ["codesign", "--verify", "--verbose", str(binary)],
            capture_output=True, text=True,
        )
        assert result.returncode == 0, (
            f"capsem-process verification failed: {result.stderr}"
        )
