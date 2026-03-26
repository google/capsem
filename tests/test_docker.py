"""Tests for Dockerfile generation from GuestImageConfig via Jinja2 templates.

TDD: these tests define the expected behavior of docker.py before implementation.
"""

import re
from pathlib import Path

import pytest

from capsem.builder.config import load_guest_config
from capsem.builder.docker import (
    GUEST_BINARIES,
    generate_build_context,
    render_dockerfile,
)

PROJECT_ROOT = Path(__file__).resolve().parent.parent


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture
def real_config():
    """Load real guest config from guest/config/."""
    return load_guest_config(PROJECT_ROOT / "guest")


@pytest.fixture
def rendered_arm64(real_config):
    return render_dockerfile("Dockerfile.rootfs.j2", real_config, "arm64")


@pytest.fixture
def rendered_x86(real_config):
    return render_dockerfile("Dockerfile.rootfs.j2", real_config, "x86_64")


# ---------------------------------------------------------------------------
# Rootfs: basic rendering
# ---------------------------------------------------------------------------


class TestRenderRootfs:
    """Rootfs Dockerfile renders correctly for both architectures."""

    def test_arm64_from_line(self, rendered_arm64):
        assert "FROM --platform=linux/arm64 debian:bookworm-slim" in rendered_arm64

    def test_x86_64_from_line(self, rendered_x86):
        assert "FROM --platform=linux/amd64 debian:bookworm-slim" in rendered_x86

    def test_apt_packages_present(self, real_config, rendered_arm64):
        for pkg in real_config.package_sets["apt"].packages:
            assert pkg in rendered_arm64, f"apt package '{pkg}' missing"

    def test_python_packages_present(self, real_config, rendered_arm64):
        for pkg in real_config.package_sets["python"].packages:
            assert pkg in rendered_arm64, f"python package '{pkg}' missing"

    def test_python_install_cmd(self, real_config, rendered_arm64):
        cmd = real_config.package_sets["python"].install_cmd
        assert cmd in rendered_arm64

    def test_npm_packages_from_providers(self, real_config, rendered_arm64):
        for provider in real_config.ai_providers.values():
            if provider.enabled and provider.install:
                for pkg in provider.install.packages:
                    assert pkg in rendered_arm64, f"npm package '{pkg}' missing"

    def test_npm_prefix(self, rendered_arm64):
        assert "/opt/ai-clis" in rendered_arm64

    def test_guest_binaries(self, rendered_arm64):
        for binary in GUEST_BINARIES:
            assert f"COPY {binary} " in rendered_arm64
            assert f"chmod 555 /usr/local/bin/{binary}" in rendered_arm64

    def test_ca_cert(self, rendered_arm64):
        assert "capsem-ca.crt" in rendered_arm64
        assert "update-ca-certificates" in rendered_arm64

    def test_certifi_patch(self, rendered_arm64):
        assert "certifi" in rendered_arm64

    def test_shell_config(self, rendered_arm64):
        assert "capsem-bashrc" in rendered_arm64
        assert "banner.txt" in rendered_arm64
        assert "tips.txt" in rendered_arm64

    def test_diagnostics(self, rendered_arm64):
        assert "diagnostics/" in rendered_arm64
        assert "capsem-doctor" in rendered_arm64
        assert "capsem-bench" in rendered_arm64

    def test_node_version(self, rendered_arm64):
        assert "nvm install 24" in rendered_arm64

    def test_uv_installed(self, rendered_arm64):
        assert "astral.sh/uv" in rendered_arm64

    def test_pep668_removal(self, rendered_arm64):
        assert "EXTERNALLY-MANAGED" in rendered_arm64

    def test_setuid_strip(self, rendered_arm64):
        assert "-4000" in rendered_arm64

    def test_root_cleanup(self, rendered_arm64):
        assert "rm -rf /root" in rendered_arm64

    def test_apt_https_switch(self, rendered_arm64):
        assert "URIs: https://" in rendered_arm64

    def test_x86_64_has_same_packages(self, real_config, rendered_x86):
        """x86_64 gets the same packages as arm64 (arch-agnostic)."""
        for pkg in real_config.package_sets["apt"].packages:
            assert pkg in rendered_x86
        for pkg in real_config.package_sets["python"].packages:
            assert pkg in rendered_x86


# ---------------------------------------------------------------------------
# Rootfs: conformance with current Dockerfile.rootfs
# ---------------------------------------------------------------------------


class TestRootfsConformance:
    """Rendered arm64 is structurally equivalent to current images/Dockerfile.rootfs."""

    @pytest.fixture
    def current_dockerfile(self):
        return (PROJECT_ROOT / "images" / "Dockerfile.rootfs").read_text()

    def test_same_from_line(self, current_dockerfile, rendered_arm64):
        assert "FROM --platform=linux/arm64 debian:bookworm-slim" in current_dockerfile
        assert "FROM --platform=linux/arm64 debian:bookworm-slim" in rendered_arm64

    def test_same_apt_packages(self, rendered_arm64):
        """All packages from apt-packages.txt appear in rendered output."""
        apt_file = (PROJECT_ROOT / "images" / "apt-packages.txt").read_text()
        pkgs = {
            ln.strip()
            for ln in apt_file.splitlines()
            if ln.strip() and not ln.strip().startswith("#")
        }
        for pkg in pkgs:
            assert pkg in rendered_arm64, f"apt package '{pkg}' in apt-packages.txt but not rendered"

    def test_same_npm_packages(self, rendered_arm64):
        """All packages from npm-globals.txt appear in rendered output."""
        npm_file = (PROJECT_ROOT / "images" / "npm-globals.txt").read_text()
        pkgs = {
            ln.strip()
            for ln in npm_file.splitlines()
            if ln.strip() and not ln.strip().startswith("#")
        }
        for pkg in pkgs:
            assert pkg in rendered_arm64, f"npm package '{pkg}' in npm-globals.txt but not rendered"

    def test_same_python_packages(self, rendered_arm64):
        """All packages from requirements.txt appear in rendered output."""
        req_file = (PROJECT_ROOT / "images" / "requirements.txt").read_text()
        pkgs = {
            ln.strip()
            for ln in req_file.splitlines()
            if ln.strip() and not ln.strip().startswith("#")
        }
        for pkg in pkgs:
            assert pkg in rendered_arm64, f"python package '{pkg}' in requirements.txt but not rendered"

    def test_same_guest_binaries(self, current_dockerfile, rendered_arm64):
        """All COPY'd capsem-* binaries in current appear in rendered."""
        current_copies = set(re.findall(r"COPY (capsem-\S+)", current_dockerfile))
        for binary in current_copies:
            assert binary in rendered_arm64, f"binary '{binary}' in current but not rendered"

    def test_same_hardening(self, current_dockerfile, rendered_arm64):
        markers = [
            "EXTERNALLY-MANAGED",
            "-4000",
            "rm -rf /root",
            "URIs: https://",
        ]
        for marker in markers:
            assert (marker in current_dockerfile) == (
                marker in rendered_arm64
            ), f"hardening marker '{marker}' mismatch"


# ---------------------------------------------------------------------------
# Kernel Dockerfile
# ---------------------------------------------------------------------------


class TestRenderKernel:
    """Kernel Dockerfile renders correctly for both architectures."""

    def test_arm64_from(self, real_config):
        result = render_dockerfile(
            "Dockerfile.kernel.j2", real_config, "arm64", kernel_version="6.6.127"
        )
        assert "FROM --platform=linux/arm64" in result

    def test_arm64_defconfig(self, real_config):
        result = render_dockerfile(
            "Dockerfile.kernel.j2", real_config, "arm64", kernel_version="6.6.127"
        )
        assert "defconfig.arm64" in result

    def test_arm64_kernel_image(self, real_config):
        result = render_dockerfile(
            "Dockerfile.kernel.j2", real_config, "arm64", kernel_version="6.6.127"
        )
        assert "arch/arm64/boot/Image" in result

    def test_x86_64_from(self, real_config):
        result = render_dockerfile(
            "Dockerfile.kernel.j2", real_config, "x86_64", kernel_version="6.6.127"
        )
        assert "FROM --platform=linux/amd64" in result

    def test_x86_64_defconfig(self, real_config):
        result = render_dockerfile(
            "Dockerfile.kernel.j2", real_config, "x86_64", kernel_version="6.6.127"
        )
        assert "defconfig.x86_64" in result

    def test_x86_64_kernel_image(self, real_config):
        result = render_dockerfile(
            "Dockerfile.kernel.j2", real_config, "x86_64", kernel_version="6.6.127"
        )
        assert "arch/x86_64/boot/bzImage" in result

    def test_kernel_version_in_url(self, real_config):
        result = render_dockerfile(
            "Dockerfile.kernel.j2", real_config, "arm64", kernel_version="6.6.128"
        )
        assert "6.6.128" in result
        assert "kernel.org" in result

    def test_busybox_and_initrd(self, real_config):
        result = render_dockerfile(
            "Dockerfile.kernel.j2", real_config, "arm64", kernel_version="6.6.127"
        )
        assert "busybox" in result
        assert "initrd" in result.lower()

    def test_capsem_init_copied(self, real_config):
        result = render_dockerfile(
            "Dockerfile.kernel.j2", real_config, "arm64", kernel_version="6.6.127"
        )
        assert "capsem-init" in result


# ---------------------------------------------------------------------------
# Kernel: conformance with current Dockerfile.kernel
# ---------------------------------------------------------------------------


class TestKernelConformance:
    """Rendered arm64 kernel is structurally equivalent to current images/Dockerfile.kernel."""

    @pytest.fixture
    def current_kernel(self):
        return (PROJECT_ROOT / "images" / "Dockerfile.kernel").read_text()

    @pytest.fixture
    def rendered_kernel(self, real_config):
        return render_dockerfile(
            "Dockerfile.kernel.j2", real_config, "arm64", kernel_version="6.6.127"
        )

    def test_same_from_line(self, current_kernel, rendered_kernel):
        assert "FROM --platform=linux/arm64 debian:bookworm-slim" in current_kernel
        assert "FROM --platform=linux/arm64 debian:bookworm-slim" in rendered_kernel

    def test_same_build_tools(self, current_kernel, rendered_kernel):
        """Core build tools are present in both."""
        tools = ["build-essential", "bc", "bison", "flex", "libssl-dev", "libelf-dev"]
        for tool in tools:
            assert tool in rendered_kernel, f"build tool '{tool}' missing from rendered"

    def test_same_defconfig(self, current_kernel, rendered_kernel):
        assert "defconfig.arm64" in current_kernel
        assert "defconfig.arm64" in rendered_kernel

    def test_same_kernel_image_path(self, current_kernel, rendered_kernel):
        assert "arch/arm64/boot/Image" in current_kernel
        assert "arch/arm64/boot/Image" in rendered_kernel

    def test_busybox_commands(self, current_kernel, rendered_kernel):
        """Both set up the same busybox symlinks."""
        assert "busybox" in current_kernel
        assert "busybox" in rendered_kernel

    def test_multistage_output(self, current_kernel, rendered_kernel):
        """Both use FROM scratch AS output stage."""
        assert "FROM scratch" in current_kernel
        assert "FROM scratch" in rendered_kernel


# ---------------------------------------------------------------------------
# Build context generation
# ---------------------------------------------------------------------------


class TestGenerateBuildContext:
    """generate_build_context() produces correct context dicts."""

    def test_rootfs_keys(self, real_config):
        ctx = generate_build_context("Dockerfile.rootfs.j2", real_config, "arm64")
        assert "arch" in ctx
        assert "arch_name" in ctx
        assert "apt_packages" in ctx
        assert "python_packages" in ctx
        assert "npm_packages" in ctx
        assert "npm_prefix" in ctx
        assert "guest_binaries" in ctx

    def test_kernel_keys(self, real_config):
        ctx = generate_build_context(
            "Dockerfile.kernel.j2", real_config, "arm64", kernel_version="6.6.127"
        )
        assert "arch" in ctx
        assert "arch_name" in ctx
        assert "kernel_version" in ctx

    def test_rootfs_npm_all_providers(self, real_config):
        ctx = generate_build_context("Dockerfile.rootfs.j2", real_config, "arm64")
        assert "@anthropic-ai/claude-code" in ctx["npm_packages"]
        assert "@google/gemini-cli" in ctx["npm_packages"]
        assert "@openai/codex" in ctx["npm_packages"]

    def test_rootfs_arch_config(self, real_config):
        ctx = generate_build_context("Dockerfile.rootfs.j2", real_config, "arm64")
        assert ctx["arch"].docker_platform == "linux/arm64"
        assert ctx["arch_name"] == "arm64"

    def test_x86_arch_config(self, real_config):
        ctx = generate_build_context("Dockerfile.rootfs.j2", real_config, "x86_64")
        assert ctx["arch"].docker_platform == "linux/amd64"
        assert ctx["arch_name"] == "x86_64"

    def test_invalid_arch_raises(self, real_config):
        with pytest.raises(KeyError):
            generate_build_context("Dockerfile.rootfs.j2", real_config, "riscv64")

    def test_unknown_template_raises(self, real_config):
        with pytest.raises(ValueError, match="Unknown template"):
            generate_build_context("Dockerfile.unknown.j2", real_config, "arm64")


# ---------------------------------------------------------------------------
# Edge cases
# ---------------------------------------------------------------------------


class TestEdgeCases:
    """Edge cases and minimal configs."""

    def test_no_python_packages(self, real_config):
        """Removing python package set still renders."""
        from capsem.builder.models import BuildConfig, GuestImageConfig

        minimal = GuestImageConfig(
            build=real_config.build,
            package_sets={"apt": real_config.package_sets["apt"]},
        )
        result = render_dockerfile("Dockerfile.rootfs.j2", minimal, "arm64")
        assert "FROM --platform=linux/arm64" in result
        # Should not have python install section
        assert "uv pip install --system" not in result or "certifi" in result

    def test_no_ai_providers(self, real_config):
        """No AI providers means no npm install section."""
        from capsem.builder.models import GuestImageConfig

        minimal = GuestImageConfig(
            build=real_config.build,
            package_sets=real_config.package_sets,
        )
        result = render_dockerfile("Dockerfile.rootfs.j2", minimal, "arm64")
        assert "FROM --platform=linux/arm64" in result
        # npm install section should be absent
        assert "npm install -g --prefix" not in result

    def test_unknown_template_render_raises(self, real_config):
        with pytest.raises(ValueError, match="Unknown template"):
            render_dockerfile("Dockerfile.bogus.j2", real_config, "arm64")

    def test_render_is_deterministic(self, real_config):
        """Two renders with same input produce identical output."""
        a = render_dockerfile("Dockerfile.rootfs.j2", real_config, "arm64")
        b = render_dockerfile("Dockerfile.rootfs.j2", real_config, "arm64")
        assert a == b
