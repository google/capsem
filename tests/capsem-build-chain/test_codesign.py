"""Verify codesigning succeeds on all daemon binaries."""

import os
import subprocess

import pytest

pytestmark = pytest.mark.build_chain


@pytest.mark.skipif(os.uname().sysname != "Darwin", reason="macOS only")
class TestCodesign:

    def test_all_signed(self, signed_binaries):
        """All daemon binaries pass codesign --verify."""
        for name, path in signed_binaries.items():
            if not path.exists():
                continue
            result = subprocess.run(
                ["codesign", "--verify", "--verbose", str(path)],
                capture_output=True, text=True,
            )
            assert result.returncode == 0, (
                f"{name} failed verification: {result.stderr}"
            )

    def test_process_has_virtualization_entitlement(self, signed_binaries):
        """capsem-process has the virtualization entitlement."""
        process = signed_binaries["capsem-process"]
        if not process.exists():
            pytest.skip("capsem-process not built")
        result = subprocess.run(
            ["codesign", "-d", "--entitlements", "-", "--xml", str(process)],
            capture_output=True, text=True,
        )
        combined = result.stdout + result.stderr
        assert "com.apple.security.virtualization" in combined, (
            "Missing virtualization entitlement on capsem-process"
        )
