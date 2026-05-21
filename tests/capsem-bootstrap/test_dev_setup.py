"""Doctor and setup sentinel tests."""

import subprocess
import tomllib

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

    def test_pyproject_exposes_capsem_admin_script(self):
        pyproject = tomllib.loads((PROJECT_ROOT / "pyproject.toml").read_text())

        scripts = pyproject["project"]["scripts"]
        assert scripts["capsem-admin"] == "capsem.admin.cli:main"

    def test_bootstrap_smokes_capsem_admin_after_uv_sync(self):
        bootstrap = (PROJECT_ROOT / "bootstrap.sh").read_text()

        assert "uv run capsem-admin --version" in bootstrap
        assert bootstrap.index("uv sync") < bootstrap.index(
            "uv run capsem-admin --version"
        )

    def test_bootstrap_installs_shared_agent_skill_symlinks_non_destructively(self):
        bootstrap = (PROJECT_ROOT / "bootstrap.sh").read_text()

        assert "install_agent_skill_links" in bootstrap
        assert '[ -e "$skill_link" ] && [ ! -L "$skill_link" ]' in bootstrap
        assert 'ln -s ../skills "$skill_link"' in bootstrap
        for agent_dir in [".claude", ".agents", ".gemini", ".codex", ".cursor"]:
            assert agent_dir in bootstrap

    def test_repo_shared_agent_skill_symlinks_point_to_skills_root(self):
        for agent_dir in [".claude", ".agents", ".gemini", ".codex", ".cursor"]:
            link = PROJECT_ROOT / agent_dir / "skills"
            assert link.is_symlink(), f"{agent_dir}/skills should be a symlink"
            assert link.readlink() == Path("../skills")

    def test_capsem_admin_entrypoint_runs_from_uv_environment(self):
        result = subprocess.run(
            ["uv", "run", "capsem-admin", "--version"],
            cwd=PROJECT_ROOT,
            capture_output=True,
            text=True,
            timeout=30,
            check=False,
        )

        assert result.returncode == 0, result.stderr
        assert "capsem-admin" in result.stdout
