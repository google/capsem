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

    def test_bootstrap_installs_linux_build_prereqs_before_cargo_tools(self):
        bootstrap = (PROJECT_ROOT / "bootstrap.sh").read_text()

        assert "build-essential" in bootstrap
        assert "nodejs npm" in bootstrap
        assert "sqlite3" in bootstrap
        assert "pkg-config" in bootstrap
        assert "libssl-dev" in bootstrap
        assert "libgtk-3-dev" in bootstrap
        assert "libwebkit2gtk-4.1-dev" in bootstrap
        assert "libayatana-appindicator3-dev" in bootstrap
        assert "librsvg2-dev" in bootstrap
        assert "libxdo-dev" in bootstrap
        assert "command -v cc" in bootstrap
        assert bootstrap.index("build-essential") < bootstrap.index(
            '"$SCRIPT_DIR/scripts/doctor-common.sh" --fix'
        )

    def test_bootstrap_exports_pnpm_bin_after_installer(self):
        bootstrap = (PROJECT_ROOT / "bootstrap.sh").read_text()

        assert 'SHELL=/bin/bash PNPM_VERSION=10.33.4 PNPM_HOME="$HOME/.local/share/pnpm"' in bootstrap
        assert 'export PATH="$PNPM_HOME:$PNPM_HOME/bin:$PATH"' in bootstrap

    def test_doctor_fix_builds_host_arch_assets(self):
        doctor = (PROJECT_ROOT / "scripts" / "doctor-common.sh").read_text()

        assert r"HOST_ARCH=\$(uname -m" in doctor
        assert r'just build-assets \"\$HOST_ARCH\"' in doctor
        assert 'CAPSEM_SKIP_ASSET_CHECK=1 just build-assets"' not in doctor

    def test_bootstrap_repairs_linux_kvm_devices_before_doctor(self):
        bootstrap = (PROJECT_ROOT / "bootstrap.sh").read_text()

        assert "scripts/fix-linux-kvm-devices.sh" in bootstrap
        assert "/dev/vhost-vsock" in bootstrap
        assert bootstrap.index("fix-linux-kvm-devices.sh") < bootstrap.index(
            '"$SCRIPT_DIR/scripts/doctor-common.sh" --fix'
        )

    def test_doctor_can_auto_fix_linux_kvm_devices(self):
        doctor = (PROJECT_ROOT / "scripts" / "doctor-common.sh").read_text()
        linux = (PROJECT_ROOT / "scripts" / "doctor-linux.sh").read_text()
        fixer = (PROJECT_ROOT / "scripts" / "fix-linux-kvm-devices.sh").read_text()

        assert "_reg linux-kvm-devices" in doctor
        assert "fixable linux-kvm-devices" in linux
        assert "/dev/vhost-vsock" in linux
        assert "modprobe vhost_vsock" in fixer
        assert 'KERNEL=="kvm"' in fixer
        assert 'KERNEL=="vhost-vsock"' in fixer

    def test_doctor_smokes_linux_kvm_device_ioctls(self):
        linux = (PROJECT_ROOT / "scripts" / "doctor-linux.sh").read_text()

        assert "KVM_GET_API_VERSION" in linux
        assert "0xAE00" in linux
        assert "KVM API usable" in linux
        assert 'os.open("/dev/kvm"' in linux
        assert 'os.open("/dev/vhost-vsock"' in linux

    def test_guest_doctor_checks_smp_visibility(self):
        environment = (
            PROJECT_ROOT / "guest" / "artifacts" / "diagnostics" / "test_environment.py"
        ).read_text()

        assert "test_smp_vcpus_visible" in environment
        assert "nproc" in environment
        assert "/proc/cpuinfo" in environment
        assert "expected at least 2 vCPUs" in environment

    def test_doctor_can_auto_fix_linux_host_build_deps(self):
        doctor = (PROJECT_ROOT / "scripts" / "doctor-common.sh").read_text()
        linux = (PROJECT_ROOT / "scripts" / "doctor-linux.sh").read_text()

        assert "_reg linux-host-build-deps" in doctor
        assert "pkg-config libssl-dev" in doctor
        assert "libgtk-3-dev" in doctor
        assert "pkgconf-pkg-config openssl-devel" in doctor
        assert "gtk3-devel" in doctor
        assert "fixable linux-host-build-deps" in linux
        assert "pkg-config --exists openssl gtk+-3.0 webkit2gtk-4.1" in linux
        assert "/usr/include/xdo.h" in linux

    def test_dev_service_refreshes_local_profile_after_asset_repack(self):
        justfile = (PROJECT_ROOT / "justfile").read_text()

        assert 'CAPSEM_ASSETS_DIR="${CAPSEM_ASSETS_DIR:-$DEV_ASSETS}"' in justfile
        assert "setup --non-interactive --accept-detected" in justfile
        assert justfile.index('CAPSEM_ASSETS_DIR="${CAPSEM_ASSETS_DIR:-$DEV_ASSETS}"') < justfile.index(
            "Starting capsem-service"
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
