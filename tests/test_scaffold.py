"""Tests for capsem.builder.scaffold -- image scaffolding and new_image.

TDD: tests written first (RED), then scaffold.py makes them pass (GREEN).
"""

from __future__ import annotations

import shutil
from pathlib import Path

import pytest

from capsem.builder.scaffold import (
    new_image,
    scan_base_config,
)

PROJECT_ROOT = Path(__file__).resolve().parent.parent


# ---------------------------------------------------------------------------
# scan_base_config
# ---------------------------------------------------------------------------


class TestScanBaseConfig:
    def test_scans_real_guest(self):
        """Scanning real guest/ config finds all providers, packages, MCP."""
        result = scan_base_config(PROJECT_ROOT / "guest")
        assert "anthropic" in result["providers"]
        assert "google" in result["providers"]
        assert "openai" in result["providers"]
        assert "apt" in result["packages"]
        assert "python" in result["packages"]
        assert "capsem" in result["mcp"]

    def test_provider_has_name(self):
        result = scan_base_config(PROJECT_ROOT / "guest")
        assert "Anthropic" in result["providers"]["anthropic"]

    def test_package_has_count(self):
        result = scan_base_config(PROJECT_ROOT / "guest")
        # apt has 14+ packages, description should mention count
        assert "package" in result["packages"]["apt"].lower()

    def test_mcp_has_description(self):
        result = scan_base_config(PROJECT_ROOT / "guest")
        assert len(result["mcp"]["capsem"]) > 0

    def test_empty_dir(self, tmp_path):
        config = tmp_path / "config"
        config.mkdir()
        (config / "build.toml").write_text("[build]\n")
        result = scan_base_config(tmp_path)
        assert result["providers"] == {}
        assert result["packages"] == {}
        assert result["mcp"] == {}

    def test_has_security(self):
        result = scan_base_config(PROJECT_ROOT / "guest")
        assert result["has_security"] is True

    def test_has_vm(self):
        result = scan_base_config(PROJECT_ROOT / "guest")
        assert result["has_vm"] is True


# ---------------------------------------------------------------------------
# new_image -- non-interactive (all defaults)
# ---------------------------------------------------------------------------


class TestNewImageAll:
    """new_image with include_*=None (all) copies everything from base."""

    def test_creates_config_dir(self, tmp_path):
        target = tmp_path / "my-image"
        new_image(target, PROJECT_ROOT / "guest", name="my-image")
        assert (target / "config").is_dir()

    def test_creates_manifest(self, tmp_path):
        target = tmp_path / "my-image"
        new_image(target, PROJECT_ROOT / "guest", name="my-image", version="1.0.0")
        manifest = target / "config" / "manifest.toml"
        assert manifest.is_file()
        content = manifest.read_text()
        assert 'name = "my-image"' in content
        assert 'version = "1.0.0"' in content

    def test_copies_build_toml(self, tmp_path):
        target = tmp_path / "my-image"
        new_image(target, PROJECT_ROOT / "guest", name="test")
        assert (target / "config" / "build.toml").is_file()

    def test_copies_kernel_defconfigs(self, tmp_path):
        target = tmp_path / "my-image"
        new_image(target, PROJECT_ROOT / "guest", name="test")
        assert (target / "config" / "kernel" / "defconfig.arm64").is_file()
        assert (target / "config" / "kernel" / "defconfig.x86_64").is_file()

    def test_copies_all_providers(self, tmp_path):
        target = tmp_path / "my-image"
        new_image(target, PROJECT_ROOT / "guest", name="test")
        ai_dir = target / "config" / "ai"
        assert (ai_dir / "anthropic.toml").is_file()
        assert (ai_dir / "google.toml").is_file()
        assert (ai_dir / "openai.toml").is_file()

    def test_copies_all_packages(self, tmp_path):
        target = tmp_path / "my-image"
        new_image(target, PROJECT_ROOT / "guest", name="test")
        pkg_dir = target / "config" / "packages"
        assert (pkg_dir / "apt.toml").is_file()
        assert (pkg_dir / "python.toml").is_file()

    def test_copies_mcp(self, tmp_path):
        target = tmp_path / "my-image"
        new_image(target, PROJECT_ROOT / "guest", name="test")
        assert (target / "config" / "mcp" / "capsem.toml").is_file()

    def test_copies_security(self, tmp_path):
        target = tmp_path / "my-image"
        new_image(target, PROJECT_ROOT / "guest", name="test")
        assert (target / "config" / "security" / "web.toml").is_file()

    def test_copies_vm_config(self, tmp_path):
        target = tmp_path / "my-image"
        new_image(target, PROJECT_ROOT / "guest", name="test")
        assert (target / "config" / "vm" / "resources.toml").is_file()
        assert (target / "config" / "vm" / "environment.toml").is_file()

    def test_result_is_loadable(self, tmp_path):
        """Created config can be loaded by load_guest_config."""
        from capsem.builder.config import load_guest_config

        target = tmp_path / "my-image"
        new_image(target, PROJECT_ROOT / "guest", name="test-image", version="2.0.0")
        config = load_guest_config(target)
        assert config.manifest is not None
        assert config.manifest.name == "test-image"
        assert config.manifest.version == "2.0.0"
        assert "anthropic" in config.ai_providers


# ---------------------------------------------------------------------------
# new_image -- selective
# ---------------------------------------------------------------------------


class TestNewImageSelective:
    """new_image with specific selections."""

    def test_select_one_provider(self, tmp_path):
        target = tmp_path / "corp"
        new_image(
            target, PROJECT_ROOT / "guest",
            name="corp", include_providers=["anthropic"],
        )
        ai_dir = target / "config" / "ai"
        assert (ai_dir / "anthropic.toml").is_file()
        assert not (ai_dir / "google.toml").exists()
        assert not (ai_dir / "openai.toml").exists()

    def test_select_no_providers(self, tmp_path):
        target = tmp_path / "minimal"
        new_image(
            target, PROJECT_ROOT / "guest",
            name="minimal", include_providers=[],
        )
        ai_dir = target / "config" / "ai"
        assert not ai_dir.exists() or len(list(ai_dir.glob("*.toml"))) == 0

    def test_exclude_security(self, tmp_path):
        target = tmp_path / "nosec"
        new_image(
            target, PROJECT_ROOT / "guest",
            name="nosec", include_security=False,
        )
        assert not (target / "config" / "security").exists()

    def test_exclude_vm(self, tmp_path):
        target = tmp_path / "novm"
        new_image(
            target, PROJECT_ROOT / "guest",
            name="novm", include_vm=False,
        )
        assert not (target / "config" / "vm").exists()

    def test_force_overwrites(self, tmp_path):
        target = tmp_path / "img"
        new_image(target, PROJECT_ROOT / "guest", name="v1")
        # Should fail without force
        with pytest.raises(FileExistsError):
            new_image(target, PROJECT_ROOT / "guest", name="v2")
        # Should succeed with force
        new_image(target, PROJECT_ROOT / "guest", name="v2", force=True)
        content = (target / "config" / "manifest.toml").read_text()
        assert 'name = "v2"' in content

    def test_manifest_has_changelog(self, tmp_path):
        target = tmp_path / "img"
        new_image(
            target, PROJECT_ROOT / "guest",
            name="test", version="0.1.0",
            description="Test image",
        )
        content = (target / "config" / "manifest.toml").read_text()
        assert "[[image.changelog]]" in content
        assert "Initial image" in content
