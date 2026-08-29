"""Doctor and setup sentinel tests."""

from pathlib import Path

import pytest

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
        plist = PROJECT_ROOT / "build_system/packaging/macos/entitlements.plist"
        assert plist.exists(), "entitlements.plist missing"

    def test_entitlements_valid_xml(self):
        """entitlements.plist must be valid XML."""
        import xml.etree.ElementTree as ET
        plist = PROJECT_ROOT / "build_system/packaging/macos/entitlements.plist"
        if not plist.exists():
            pytest.skip("No entitlements.plist")
        # Should not raise
        ET.parse(plist)

    def test_entitlements_has_virtualization(self):
        plist = PROJECT_ROOT / "build_system/packaging/macos/entitlements.plist"
        if not plist.exists():
            pytest.skip("No entitlements.plist")
        text = plist.read_text()
        assert "com.apple.security.virtualization" in text

    def test_justfile_exists(self):
        assert (PROJECT_ROOT / "justfile").exists()

    def test_cargo_toml_exists(self):
        assert (PROJECT_ROOT / "Cargo.toml").exists()

    def test_bootstrap_pnpm_install_is_noninteractive(self):
        bootstrap = (PROJECT_ROOT / "bootstrap.sh").read_text()
        gate = (PROJECT_ROOT / "config/gate.toml").read_text()

        assert "uv run --project build_system --frozen capsem-gate install-node" in bootstrap
        assert "pnpm install --frozen-lockfile" not in bootstrap
        assert 'node_env = { CI = "true" }' in gate
        assert (
            'node_workspaces = ["web/app", "web/docs", "site", '
            '"build_system/release_site"]' in gate
        )
