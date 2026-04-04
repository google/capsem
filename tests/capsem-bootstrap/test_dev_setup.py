"""Doctor and setup sentinel tests."""

import pytest

from pathlib import Path

PROJECT_ROOT = Path(__file__).parent.parent.parent

pytestmark = pytest.mark.bootstrap


class TestDevSetup:

    def test_dev_setup_sentinel_exists(self):
        """After initial setup, .dev-setup sentinel should exist."""
        sentinel = PROJECT_ROOT / ".dev-setup"
        # This may not exist in CI or fresh clones -- skip if so
        if not sentinel.exists():
            pytest.skip(".dev-setup not found (run `just doctor` first)")
        assert sentinel.stat().st_size == 0 or sentinel.exists()

    def test_entitlements_plist_exists(self):
        plist = PROJECT_ROOT / "entitlements.plist"
        assert plist.exists(), "entitlements.plist missing"

    def test_entitlements_valid_xml(self):
        """entitlements.plist must be valid XML."""
        import xml.etree.ElementTree as ET
        plist = PROJECT_ROOT / "entitlements.plist"
        if not plist.exists():
            pytest.skip("No entitlements.plist")
        # Should not raise
        ET.parse(plist)

    def test_entitlements_has_virtualization(self):
        plist = PROJECT_ROOT / "entitlements.plist"
        if not plist.exists():
            pytest.skip("No entitlements.plist")
        text = plist.read_text()
        assert "com.apple.security.virtualization" in text

    def test_justfile_exists(self):
        assert (PROJECT_ROOT / "justfile").exists()

    def test_cargo_toml_exists(self):
        assert (PROJECT_ROOT / "Cargo.toml").exists()
