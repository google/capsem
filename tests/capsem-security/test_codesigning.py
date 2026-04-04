"""Codesigning and entitlements validation."""

import os
import subprocess

import pytest

from pathlib import Path

PROJECT_ROOT = Path(__file__).parent.parent.parent

pytestmark = pytest.mark.security


@pytest.fixture
def signed_process():
    """Path to capsem-process if it exists and is signed."""
    binary = PROJECT_ROOT / "target/debug/capsem-process"
    if not binary.exists():
        pytest.skip("capsem-process not built")
    return binary


def test_entitlements_present(signed_process):
    """Signed capsem-process has virtualization entitlement."""
    if os.uname().sysname != "Darwin":
        pytest.skip("macOS only")

    result = subprocess.run(
        ["codesign", "-d", "--entitlements", "-", "--xml", str(signed_process)],
        capture_output=True, text=True,
    )
    if result.returncode != 0:
        pytest.skip(f"Binary not signed: {result.stderr}")

    assert "com.apple.security.virtualization" in result.stdout or \
           "com.apple.security.virtualization" in result.stderr, \
        "Missing virtualization entitlement"


def test_entitlements_plist_valid():
    """entitlements.plist is valid XML with required keys."""
    import xml.etree.ElementTree as ET
    plist = PROJECT_ROOT / "entitlements.plist"
    assert plist.exists()
    tree = ET.parse(plist)
    text = plist.read_text()
    assert "com.apple.security.virtualization" in text
    assert "com.apple.security.network.client" in text
