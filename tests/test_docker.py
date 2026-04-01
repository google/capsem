"""Tests for Dockerfile generation and build execution from GuestImageConfig.

TDD: these tests define the expected behavior of docker.py before implementation.
Build execution tests mock run_cmd (single subprocess seam) -- no Docker needed.
"""

import json
import re
import shutil
import subprocess
from pathlib import Path
from unittest.mock import MagicMock, call, patch

import pytest

from capsem.builder.config import load_guest_config
from capsem.builder.docker import (
    GUEST_BINARIES,
    build_version_script,
    container_compile_agent,
    cross_compile_agent,
    extract_tool_versions,
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
        assert "capsem_bench/" in rendered_arm64

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


class TestRootfsLayerOrdering:
    """Dockerfile layers must be in the correct order for the build to succeed.

    Many CI failures trace back to ordering bugs: a binary referenced before
    it's COPYed, PATH set before the install, CA cert patched before it exists.
    These tests encode the required ordering as position checks on the rendered
    Dockerfile so we catch them in unit tests, not 20 min into a CI run.
    """

    def _pos(self, text, needle, label=None):
        """Return position of needle in text, with a clear assertion on miss."""
        pos = text.find(needle)
        assert pos != -1, f"Expected to find {label or repr(needle)} in Dockerfile"
        return pos

    def test_env_path_includes_npm_prefix(self, rendered_arm64):
        """Regression: v0.14.18 -- /opt/ai-clis/bin not on PATH, gemini/codex
        returned N/A, build-time validator rejected the rootfs."""
        assert 'ENV PATH="/opt/ai-clis/bin:$PATH"' in rendered_arm64, (
            "Dockerfile.rootfs.j2 must set ENV PATH to include /opt/ai-clis/bin "
            "so version extraction can find npm-installed AI CLIs"
        )

    def test_env_path_after_npm_install(self, rendered_arm64):
        npm_pos = self._pos(rendered_arm64, "npm install -g --prefix", "npm install")
        path_pos = self._pos(rendered_arm64, 'ENV PATH="/opt/ai-clis/bin', "ENV PATH")
        assert npm_pos < path_pos, "ENV PATH must come after npm install"

    def test_ca_cert_before_certifi_patch(self, rendered_arm64):
        """certifi patch appends our CA to certifi's bundle -- cert must exist first."""
        copy_ca = self._pos(rendered_arm64, "COPY capsem-ca.crt", "COPY CA cert")
        update_ca = self._pos(rendered_arm64, "update-ca-certificates", "update-ca-certificates")
        certifi_patch = self._pos(rendered_arm64, "certifi.where()", "certifi patch")
        assert copy_ca < update_ca < certifi_patch, (
            "Order must be: COPY cert -> update-ca-certificates -> certifi patch"
        )

    def test_node_before_npm_install(self, rendered_arm64):
        """npm install requires node to be installed first."""
        node_pos = self._pos(rendered_arm64, "nvm install", "node install")
        npm_pos = self._pos(rendered_arm64, "npm install -g --prefix", "npm install")
        assert node_pos < npm_pos, "Node.js must be installed before npm install"

    def test_guest_binaries_before_root_cleanup(self, rendered_arm64):
        """Guest binaries are COPYed into /usr/local/bin -- must happen before
        /root cleanup which wipes the build context landing area."""
        binary_pos = self._pos(rendered_arm64, "COPY capsem-pty-agent", "guest binary")
        cleanup_pos = self._pos(rendered_arm64, "rm -rf /root", "/root cleanup")
        assert binary_pos < cleanup_pos, "Guest binaries must be COPYed before /root cleanup"

    def test_curl_installs_after_root_cleanup(self, rendered_arm64):
        """Curl-installed CLIs (claude) write to ~/.local/bin then copy to
        /usr/local/bin. They must come after /root cleanup (mkdir -p /root)
        so the installer has a writable home directory."""
        if "curl -fsSL" not in rendered_arm64:
            pytest.skip("No curl installs in config")
        cleanup_pos = self._pos(rendered_arm64, "rm -rf /root && mkdir -p /root", "/root cleanup")
        curl_pos = self._pos(rendered_arm64, "curl -fsSL", "curl install")
        assert cleanup_pos < curl_pos, "Curl installs must come after /root cleanup"

    def test_apt_https_switch_is_last(self, rendered_arm64):
        """Switching apt to HTTPS must be the last step -- earlier RUN commands
        need HTTP apt (ca-certificates not yet installed at the start)."""
        https_pos = self._pos(rendered_arm64, "URIs: https://", "apt HTTPS switch")
        # Nothing else should be installed after the HTTPS switch
        last_run = rendered_arm64.rfind("RUN ")
        https_run = rendered_arm64.rfind("RUN ", 0, https_pos + 1)
        assert https_run == last_run, (
            "apt HTTPS switch must be in the final RUN layer -- "
            "no package installs should follow it"
        )

    def test_setuid_strip_after_all_installs(self, rendered_arm64):
        """Setuid strip must come after all package installs so no new
        setuid binaries sneak in after the strip."""
        strip_pos = self._pos(rendered_arm64, "-4000", "setuid strip")
        # Must be after npm, python, and guest binary installs
        npm_pos = self._pos(rendered_arm64, "npm install -g --prefix", "npm install")
        assert strip_pos > npm_pos, "setuid strip must come after npm install"
        if "uv pip install --system" in rendered_arm64:
            # Find the LAST uv pip install (python packages, not certifi)
            last_pip = rendered_arm64.rfind("uv pip install --system --break-system-packages")
            assert strip_pos > last_pip, "setuid strip must come after python packages"

    def test_x86_64_has_same_ordering(self, rendered_arm64, rendered_x86):
        """Both architectures must have the same layer ordering."""
        key_markers = [
            "apt-get",
            "nvm install",
            "npm install -g --prefix",
            "ENV PATH",
            "capsem-ca.crt",
            "certifi",
            "capsem-pty-agent",
            "capsem-bashrc",
            "EXTERNALLY-MANAGED",
            "-4000",
            "rm -rf /root",
            "URIs: https://",
        ]
        arm64_order = [m for m in key_markers if m in rendered_arm64]
        x86_order = [m for m in key_markers if m in rendered_x86]
        assert arm64_order == x86_order, (
            f"Layer ordering differs between arm64 and x86_64:\n"
            f"  arm64:  {arm64_order}\n"
            f"  x86_64: {x86_order}"
        )


class TestRootfsSecurityInvariants:
    """Security-critical properties of the rootfs Dockerfile."""

    def test_guest_binaries_chmod_555(self, rendered_arm64):
        """All guest binaries must be read-only (chmod 555)."""
        for binary in GUEST_BINARIES:
            assert f"chmod 555 /usr/local/bin/{binary}" in rendered_arm64, (
                f"Guest binary {binary} must be chmod 555 (read-only)"
            )

    def test_no_writable_guest_binaries(self, rendered_arm64):
        """Guest binaries must never be chmod 755 or higher."""
        for binary in GUEST_BINARIES:
            assert f"chmod 755 /usr/local/bin/{binary}" not in rendered_arm64, (
                f"Guest binary {binary} must not be writable (755) -- use 555"
            )

    def test_pep668_removed(self, rendered_arm64):
        """PEP 668 marker must be removed so pip works in the ephemeral VM."""
        assert "rm -f /usr/lib/python*/EXTERNALLY-MANAGED" in rendered_arm64

    def test_setuid_bits_stripped(self, rendered_arm64):
        """All setuid/setgid bits must be stripped -- VM runs as root."""
        assert "perm -4000" in rendered_arm64
        assert "perm -2000" in rendered_arm64
        assert "chmod u-s,g-s" in rendered_arm64

    def test_apt_sources_https(self, rendered_arm64):
        """Runtime apt must use HTTPS -- VM blocks port 80."""
        assert "URIs: https://" in rendered_arm64


class TestRootfsVersionExtractability:
    """Every tool with a version_command in config must be findable in the
    built image. This class validates that the Dockerfile installs them
    in locations that will be on PATH when extract_tool_versions runs."""

    def test_all_ai_cli_install_prefixes_on_path(self, real_config, rendered_arm64):
        """Every AI provider with an npm install prefix must have that
        prefix's bin/ on ENV PATH in the Dockerfile."""
        for provider in real_config.ai_providers.values():
            if not (provider.enabled and provider.install):
                continue
            if provider.install.manager.value == "npm" and provider.install.prefix:
                prefix_bin = f"{provider.install.prefix}/bin"
                assert prefix_bin in rendered_arm64, (
                    f"AI provider {provider.name} installs to {provider.install.prefix} "
                    f"but {prefix_bin} is not on PATH in the Dockerfile"
                )

    def test_curl_installed_clis_copied_to_usr_local(self, real_config, rendered_arm64):
        """Curl-installed CLIs write to ~/.local/bin which is tmpfs at runtime.
        The Dockerfile must copy them to /usr/local/bin."""
        has_curl = any(
            p.enabled and p.install and p.install.manager.value == "curl"
            for p in real_config.ai_providers.values()
        )
        if has_curl:
            assert 'install -m 555 "$bin" /usr/local/bin/' in rendered_arm64, (
                "Curl-installed CLIs must be copied to /usr/local/bin so they "
                "survive boot (tmpfs wipes /root/.local/bin)"
            )

    def test_system_tools_on_default_path(self, real_config, rendered_arm64):
        """System tools (node, npm, uv, git, etc.) must be symlinked or
        installed into /usr/local/bin which is on the default PATH."""
        # node/npm are symlinked by the nvm install step
        assert "ln -sf" in rendered_arm64 and "node" in rendered_arm64
        # uv is explicitly installed to /usr/local/bin
        assert "install -m 555 /root/.local/bin/uv /usr/local/bin/uv" in rendered_arm64


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

    def test_rootfs_npm_providers(self, real_config):
        ctx = generate_build_context("Dockerfile.rootfs.j2", real_config, "arm64")
        assert "@google/gemini-cli" in ctx["npm_packages"]
        assert "@openai/codex" in ctx["npm_packages"]

    def test_rootfs_curl_installs(self, real_config):
        ctx = generate_build_context("Dockerfile.rootfs.j2", real_config, "arm64")
        assert "https://claude.ai/install.sh" in ctx["curl_installs"]

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


# ---------------------------------------------------------------------------
# Build execution: resolve_kernel_version
# ---------------------------------------------------------------------------


from capsem.builder.docker import (
    FALLBACK_KERNEL_VERSION,
    create_squashfs,
    detect_runtime,
    docker_build,
    export_container_fs,
    extract_kernel_assets,
    generate_checksums,
    get_project_version,
    is_ci,
    prepare_build_context,
    resolve_kernel_version,
    run_cmd,
    sync_container_clock,
)


class TestResolveKernelVersion:
    @patch("capsem.builder.docker.urllib.request.urlopen")
    def test_valid_json(self, mock_urlopen):
        mock_resp = MagicMock()
        mock_resp.read.return_value = json.dumps({
            "releases": [
                {"version": "6.6.131", "moniker": "longterm", "iseol": False},
                {"version": "6.6.127", "moniker": "longterm", "iseol": False},
                {"version": "6.12.5", "moniker": "stable", "iseol": False},
            ]
        }).encode()
        mock_resp.__enter__ = lambda s: s
        mock_resp.__exit__ = MagicMock(return_value=False)
        mock_urlopen.return_value = mock_resp
        result = resolve_kernel_version("6.6")
        assert result == "6.6.131"

    @patch("capsem.builder.docker.urllib.request.urlopen")
    def test_network_error_fallback(self, mock_urlopen):
        mock_urlopen.side_effect = Exception("network error")
        result = resolve_kernel_version("6.6")
        assert result == FALLBACK_KERNEL_VERSION

    @patch("capsem.builder.docker.urllib.request.urlopen")
    def test_no_matching_branch(self, mock_urlopen):
        mock_resp = MagicMock()
        mock_resp.read.return_value = json.dumps({
            "releases": [
                {"version": "6.12.5", "moniker": "stable", "iseol": False},
            ]
        }).encode()
        mock_resp.__enter__ = lambda s: s
        mock_resp.__exit__ = MagicMock(return_value=False)
        mock_urlopen.return_value = mock_resp
        result = resolve_kernel_version("6.6")
        assert result == FALLBACK_KERNEL_VERSION

    @patch("capsem.builder.docker.urllib.request.urlopen")
    def test_eol_versions_skipped(self, mock_urlopen):
        mock_resp = MagicMock()
        mock_resp.read.return_value = json.dumps({
            "releases": [
                {"version": "6.6.131", "moniker": "longterm", "iseol": True},
                {"version": "6.6.127", "moniker": "longterm", "iseol": False},
            ]
        }).encode()
        mock_resp.__enter__ = lambda s: s
        mock_resp.__exit__ = MagicMock(return_value=False)
        mock_urlopen.return_value = mock_resp
        result = resolve_kernel_version("6.6")
        assert result == "6.6.127"


# ---------------------------------------------------------------------------
# Build execution: detect_runtime, is_ci
# ---------------------------------------------------------------------------


class TestDetectRuntime:
    @patch("capsem.builder.docker.check_container_runtime")
    def test_docker_found(self, mock_check):
        from capsem.builder.doctor import CheckResult
        mock_check.return_value = CheckResult(
            name="container-runtime", passed=True, detail="Docker version 27.1.1"
        )
        assert detect_runtime() == "docker"

    @patch("capsem.builder.docker.check_container_runtime")
    def test_not_found_raises(self, mock_check):
        from capsem.builder.doctor import CheckResult
        mock_check.return_value = CheckResult(
            name="container-runtime", passed=False,
            detail="docker not found", fix="brew install colima docker",
        )
        with pytest.raises(RuntimeError, match="container-runtime"):
            detect_runtime()


class TestIsCi:
    @patch.dict("os.environ", {"GITHUB_ACTIONS": "true"})
    def test_ci_true(self):
        assert is_ci() is True

    @patch.dict("os.environ", {}, clear=True)
    def test_ci_false(self):
        assert is_ci() is False


# ---------------------------------------------------------------------------
# Build execution: sync_container_clock
# ---------------------------------------------------------------------------


class TestSyncContainerClock:
    @patch("capsem.builder.docker.sys")
    @patch("capsem.builder.docker.run_cmd")
    def test_syncs_via_privileged_container(self, mock_run, mock_sys):
        mock_sys.platform = "darwin"
        sync_container_clock()
        cmd = mock_run.call_args[0][0]
        assert cmd[0] == "docker"
        assert "--privileged" in cmd
        assert "date" in cmd

    @patch("capsem.builder.docker.sys")
    @patch("capsem.builder.docker.run_cmd")
    def test_noop_on_linux(self, mock_run, mock_sys):
        mock_sys.platform = "linux"
        sync_container_clock()
        mock_run.assert_not_called()

    @patch("capsem.builder.docker.sys")
    @patch("capsem.builder.docker.run_cmd")
    def test_swallows_errors(self, mock_run, mock_sys):
        mock_sys.platform = "darwin"
        mock_run.side_effect = Exception("VM not running")
        # Should not raise
        sync_container_clock()


# ---------------------------------------------------------------------------
# Build execution: docker_build
# ---------------------------------------------------------------------------


class TestDockerBuild:
    @patch("capsem.builder.docker.run_cmd")
    def test_regular_build(self, mock_run):
        docker_build(
            runtime="docker", tag="test-image", dockerfile_path="/tmp/Dockerfile",
            context_dir="/tmp/ctx", platform="linux/arm64",
        )
        cmd = mock_run.call_args[0][0]
        assert cmd[0] == "docker"
        assert "build" in cmd
        assert "--platform" in cmd
        assert "linux/arm64" in cmd
        assert "-t" in cmd
        assert "test-image" in cmd

    @patch("capsem.builder.docker.run_cmd")
    def test_ci_docker_uses_buildx(self, mock_run):
        docker_build(
            runtime="docker", tag="test-image", dockerfile_path="/tmp/Dockerfile",
            context_dir="/tmp/ctx", platform="linux/arm64", ci_cache=True,
        )
        cmd = mock_run.call_args[0][0]
        assert cmd[:3] == ["docker", "buildx", "build"]
        assert "--cache-from" in cmd
        assert "--cache-to" in cmd
        assert "--load" in cmd

    @patch("capsem.builder.docker.run_cmd")
    def test_build_args(self, mock_run):
        docker_build(
            runtime="docker", tag="test", dockerfile_path="/tmp/Dockerfile",
            context_dir="/tmp/ctx", platform="linux/arm64",
            build_args={"KERNEL_VERSION": "6.6.131"},
        )
        cmd = mock_run.call_args[0][0]
        assert "--build-arg" in cmd
        idx = cmd.index("--build-arg")
        assert cmd[idx + 1] == "KERNEL_VERSION=6.6.131"


# ---------------------------------------------------------------------------
# Build execution: extract/export
# ---------------------------------------------------------------------------


class TestExtractKernelAssets:
    @patch("capsem.builder.docker.run_cmd")
    def test_create_cp_rm_sequence(self, mock_run):
        mock_run.return_value = MagicMock(stdout="container123\n")
        out = Path("/tmp/test-assets")
        extract_kernel_assets("docker", "kernel-img", "linux/arm64", out)
        calls = mock_run.call_args_list
        # create, cp vmlinuz, cp initrd, rm
        assert len(calls) == 4
        assert "create" in calls[0][0][0]
        assert "cp" in calls[1][0][0]
        assert "cp" in calls[2][0][0]
        assert "rm" in calls[3][0][0]


class TestExportContainerFs:
    @patch("capsem.builder.docker.run_cmd")
    def test_create_export_rm_sequence(self, mock_run):
        mock_run.return_value = MagicMock(stdout="container456\n")
        export_container_fs("docker", "rootfs-img", "linux/arm64", Path("/tmp/rootfs.tar"))
        calls = mock_run.call_args_list
        assert len(calls) == 3
        assert "create" in calls[0][0][0]
        assert "export" in calls[1][0][0]
        assert "rm" in calls[2][0][0]


# ---------------------------------------------------------------------------
# Build execution: squashfs
# ---------------------------------------------------------------------------


# ---------------------------------------------------------------------------
# Build execution: version script generation
# ---------------------------------------------------------------------------


class TestBuildVersionScript:
    """build_version_script() assembles a shell script from config."""

    def test_real_config_has_all_sections(self, real_config):
        script = build_version_script(real_config)
        assert '# System' in script
        assert '# Python' in script
        assert '# AI CLIs' in script

    def test_real_config_has_build_tools(self, real_config):
        script = build_version_script(real_config)
        assert 'node=' in script
        assert 'npm=' in script
        assert 'uv=' in script
        assert 'pip=' in script

    def test_real_config_has_apt_tools(self, real_config):
        script = build_version_script(real_config)
        assert 'git=' in script
        assert 'python3=' in script
        assert 'gh=' in script

    def test_real_config_has_python_packages(self, real_config):
        script = build_version_script(real_config)
        assert 'pytest=' in script
        assert 'numpy=' in script

    def test_real_config_has_ai_clis(self, real_config):
        script = build_version_script(real_config)
        assert 'claude=' in script
        assert 'gemini=' in script
        assert 'codex=' in script

    def test_empty_config_produces_empty_script(self):
        from capsem.builder.models import BuildConfig, GuestImageConfig
        config = GuestImageConfig(
            build=BuildConfig(architectures={"arm64": real_arch()}),
        )
        script = build_version_script(config)
        assert script == ""

    def test_disabled_provider_excluded(self, real_config):
        """Disabled AI providers are not included in the version script."""
        from capsem.builder.models import GuestImageConfig
        # Create config with all providers disabled
        disabled_providers = {}
        for key, prov in real_config.ai_providers.items():
            disabled_providers[key] = prov.model_copy(update={"enabled": False})
        config = real_config.model_copy(update={"ai_providers": disabled_providers})
        script = build_version_script(config)
        assert "# AI CLIs" not in script


def real_arch():
    """Minimal ArchConfig for test configs."""
    from capsem.builder.models import ArchConfig
    return ArchConfig(
        docker_platform="linux/arm64",
        rust_target="aarch64-unknown-linux-musl",
        kernel_image="arch/arm64/boot/Image",
        defconfig="kernel/defconfig.arm64",
    )


class TestExtractToolVersionsValidation:
    """extract_tool_versions() validates AI CLI results."""

    @patch("capsem.builder.docker.run_cmd")
    def test_valid_output_passes(self, mock_run, real_config):
        mock_run.return_value = MagicMock(stdout=(
            "# System\n"
            "node=24.1.0\nnpm=10.9.2\nuv=0.7.12\npip=24.0\n"
            "python3=3.11.2\ngit=2.39.5\ngh=2.67.0\ntmux=3.4\ncurl=7.88.1\n"
            "# Python\n"
            "pytest=8.3.4\nnumpy=2.2.3\nrequests=2.32.3\npandas=2.2.3\n"
            "# AI CLIs\n"
            "claude=1.0.18\ngemini=0.3.0\ncodex=0.1.0\n"
        ))
        # Should not raise
        extract_tool_versions(
            "docker", "test-image", "linux/arm64",
            Path("/tmp"), real_config,
        )

    @patch("capsem.builder.docker.run_cmd")
    def test_na_ai_cli_raises(self, mock_run, real_config):
        mock_run.return_value = MagicMock(stdout=(
            "# System\n"
            "node=24.1.0\n"
            "# AI CLIs\n"
            "claude=1.0.18\ngemini=N/A\ncodex=N/A\n"
        ))
        with pytest.raises(RuntimeError, match="gemini"):
            extract_tool_versions(
                "docker", "test-image", "linux/arm64",
                Path("/tmp"), real_config,
            )

    @patch("capsem.builder.docker.run_cmd")
    def test_validate_false_skips_check(self, mock_run, real_config):
        mock_run.return_value = MagicMock(stdout=(
            "# AI CLIs\n"
            "claude=N/A\ngemini=N/A\ncodex=N/A\n"
        ))
        # Should not raise when validate=False
        extract_tool_versions(
            "docker", "test-image", "linux/arm64",
            Path("/tmp"), real_config, validate=False,
        )


# ---------------------------------------------------------------------------
# Clock-skew resilience: apt-get must bypass both date checks
# ---------------------------------------------------------------------------


class TestAptClockSkewOptions:
    """Every apt-get update must include both Check-Valid-Until=false and
    Check-Date=false so builds survive Colima VM clock drift.

    Regression test for: container VM clock behind real time causes
    "Release file is not valid yet" errors even with Check-Valid-Until=false.
    """

    APT_CLOCK_SKEW_OPTIONS = [
        "Acquire::Check-Valid-Until=false",
        "Acquire::Check-Date=false",
    ]

    def test_rootfs_template_has_both_options(self, rendered_arm64):
        for opt in self.APT_CLOCK_SKEW_OPTIONS:
            assert opt in rendered_arm64, (
                f"Dockerfile.rootfs.j2 missing apt option '{opt}' -- "
                "builds will fail when container clock drifts"
            )

    def test_kernel_template_has_both_options(self, real_config):
        rendered = render_dockerfile(
            "Dockerfile.kernel.j2", real_config, "arm64", kernel_version="6.6.127"
        )
        for opt in self.APT_CLOCK_SKEW_OPTIONS:
            assert opt in rendered, (
                f"Dockerfile.kernel.j2 missing apt option '{opt}' -- "
                "builds will fail when container clock drifts"
            )

    @patch("capsem.builder.docker.run_cmd")
    def test_create_squashfs_has_both_options(self, mock_run):
        create_squashfs(
            "docker", Path("/tmp/rootfs.tar"), Path("/tmp/rootfs.squashfs"),
            "zstd", 15,
        )
        cmd_str = " ".join(mock_run.call_args[0][0])
        for opt in self.APT_CLOCK_SKEW_OPTIONS:
            assert opt in cmd_str, (
                f"create_squashfs() missing apt option '{opt}' -- "
                "squashfs builds will fail when container clock drifts"
            )


class TestCreateSquashfs:
    @patch("capsem.builder.docker.run_cmd")
    def test_zstd_compression(self, mock_run):
        create_squashfs(
            "docker", Path("/tmp/rootfs.tar"), Path("/tmp/rootfs.squashfs"),
            "zstd", 15,
        )
        cmd = mock_run.call_args[0][0]
        cmd_str = " ".join(cmd)
        assert "mksquashfs" in cmd_str
        assert "-comp zstd" in cmd_str
        assert "-Xcompression-level 15" in cmd_str

    @patch("capsem.builder.docker.run_cmd")
    def test_gzip_no_level_flag(self, mock_run):
        create_squashfs(
            "docker", Path("/tmp/rootfs.tar"), Path("/tmp/rootfs.squashfs"),
            "gzip", 9,
        )
        cmd = mock_run.call_args[0][0]
        cmd_str = " ".join(cmd)
        assert "mksquashfs" in cmd_str
        assert "-comp gzip" in cmd_str
        # gzip doesn't support -Xcompression-level in mksquashfs
        assert "-Xcompression-level" not in cmd_str


# ---------------------------------------------------------------------------
# Build execution: prepare_build_context
# ---------------------------------------------------------------------------


class TestPrepareBuildContext:
    def test_rootfs_context_has_all_files(self, real_config, tmp_path):
        context_dir = tmp_path / "ctx"
        context_dir.mkdir()
        prepare_build_context(
            real_config, "arm64", "Dockerfile.rootfs.j2",
            context_dir, PROJECT_ROOT,
        )
        # Dockerfile
        assert (context_dir / "Dockerfile").is_file()
        # CA cert
        assert (context_dir / "capsem-ca.crt").is_file()
        # Shell config
        assert (context_dir / "capsem-bashrc").is_file()
        assert (context_dir / "banner.txt").is_file()
        assert (context_dir / "tips.txt").is_file()
        # Diagnostics
        assert (context_dir / "diagnostics").is_dir()
        assert (context_dir / "capsem-doctor").is_file()
        assert (context_dir / "capsem-bench").is_file()
        assert (context_dir / "capsem_bench").is_dir()
        assert (context_dir / "capsem_bench" / "__main__.py").is_file()

    def test_kernel_context_has_defconfig_and_init(self, real_config, tmp_path):
        context_dir = tmp_path / "ctx"
        context_dir.mkdir()
        prepare_build_context(
            real_config, "arm64", "Dockerfile.kernel.j2",
            context_dir, PROJECT_ROOT, kernel_version="6.6.127",
        )
        assert (context_dir / "Dockerfile").is_file()
        assert (context_dir / "kernel" / "defconfig.arm64").is_file()
        assert (context_dir / "capsem-init").is_file()

    def test_rootfs_dockerfile_content(self, real_config, tmp_path):
        context_dir = tmp_path / "ctx"
        context_dir.mkdir()
        prepare_build_context(
            real_config, "arm64", "Dockerfile.rootfs.j2",
            context_dir, PROJECT_ROOT,
        )
        content = (context_dir / "Dockerfile").read_text()
        assert "FROM --platform=linux/arm64" in content

    def test_kernel_dockerfile_has_version(self, real_config, tmp_path):
        context_dir = tmp_path / "ctx"
        context_dir.mkdir()
        prepare_build_context(
            real_config, "arm64", "Dockerfile.kernel.j2",
            context_dir, PROJECT_ROOT, kernel_version="6.6.131",
        )
        content = (context_dir / "Dockerfile").read_text()
        assert "6.6.131" in content


# ---------------------------------------------------------------------------
# Build execution: get_project_version
# ---------------------------------------------------------------------------


class TestGetProjectVersion:
    def test_reads_real_cargo_toml(self):
        version = get_project_version(PROJECT_ROOT)
        assert re.match(r"\d+\.\d+\.\d+", version)

    def test_missing_cargo_toml_raises(self, tmp_path):
        with pytest.raises(RuntimeError):
            get_project_version(tmp_path)


# ---------------------------------------------------------------------------
# Build execution: generate_checksums
# ---------------------------------------------------------------------------


class TestGenerateChecksums:
    def test_b3sum_and_manifest(self, tmp_path):
        # Create fake arch dirs with files
        arm64 = tmp_path / "arm64"
        arm64.mkdir()
        (arm64 / "vmlinuz").write_bytes(b"kernel")
        (arm64 / "initrd.img").write_bytes(b"initrd")
        (arm64 / "rootfs.squashfs").write_bytes(b"rootfs")
        generate_checksums(tmp_path, "0.13.0")
        # B3SUMS was written
        assert (tmp_path / "B3SUMS").exists()
        # manifest.json was written
        manifest = json.loads((tmp_path / "manifest.json").read_text())
        assert manifest["latest"] == "0.13.0"
        assert "0.13.0" in manifest["releases"]

    def test_manifest_per_arch_structure(self, tmp_path):
        """Per-arch layout produces release[arch][assets] with bare filenames.

        build.rs looks up: releases[version][arch_key]["assets"][i]["filename"]
        where filename must be bare (e.g. "vmlinuz", NOT "arm64/vmlinuz").
        """
        arm64 = tmp_path / "arm64"
        arm64.mkdir()
        (arm64 / "vmlinuz").write_bytes(b"kernel")
        (arm64 / "initrd.img").write_bytes(b"initrd")
        (arm64 / "rootfs.squashfs").write_bytes(b"rootfs")
        generate_checksums(tmp_path, "0.13.0")
        manifest = json.loads((tmp_path / "manifest.json").read_text())
        release = manifest["releases"]["0.13.0"]
        assert "arm64" in release, "per-arch key 'arm64' missing from release"
        arm64_assets = release["arm64"]["assets"]
        filenames = {a["filename"] for a in arm64_assets}
        assert filenames == {"vmlinuz", "initrd.img", "rootfs.squashfs"}
        # No filename should contain '/' (build.rs matches bare names)
        for asset in arm64_assets:
            assert "/" not in asset["filename"], f"bare filename expected, got: {asset['filename']}"
            assert len(asset["hash"]) == 64  # blake3 hex digest
            assert asset["size"] > 0

    def test_manifest_flat_fallback(self, tmp_path):
        """When no arch subdirs exist, produces flat format with bare filenames."""
        (tmp_path / "vmlinuz").write_bytes(b"kernel")
        (tmp_path / "initrd.img").write_bytes(b"initrd")
        (tmp_path / "rootfs.squashfs").write_bytes(b"rootfs")
        generate_checksums(tmp_path, "0.13.0")
        manifest = json.loads((tmp_path / "manifest.json").read_text())
        release = manifest["releases"]["0.13.0"]
        assert "assets" in release
        filenames = {a["filename"] for a in release["assets"]}
        assert filenames == {"vmlinuz", "initrd.img", "rootfs.squashfs"}

    def test_manifest_multi_arch(self, tmp_path):
        """Both arm64 and x86_64 subdirs produce both arch keys."""
        for arch in ("arm64", "x86_64"):
            d = tmp_path / arch
            d.mkdir()
            (d / "vmlinuz").write_bytes(f"kernel-{arch}".encode())
            (d / "initrd.img").write_bytes(f"initrd-{arch}".encode())
            (d / "rootfs.squashfs").write_bytes(f"rootfs-{arch}".encode())
        generate_checksums(tmp_path, "0.13.0")
        manifest = json.loads((tmp_path / "manifest.json").read_text())
        release = manifest["releases"]["0.13.0"]
        assert "arm64" in release
        assert "x86_64" in release
        assert len(release["arm64"]["assets"]) == 3
        assert len(release["x86_64"]["assets"]) == 3
        # Each arch has distinct hashes
        arm_hashes = {a["hash"] for a in release["arm64"]["assets"]}
        x86_hashes = {a["hash"] for a in release["x86_64"]["assets"]}
        assert arm_hashes.isdisjoint(x86_hashes)


# ---------------------------------------------------------------------------
# Build execution: agent compilation
# ---------------------------------------------------------------------------


class TestContainerCompileAgent:
    """Tests for container_compile_agent() -- single-container build."""

    @patch("capsem.builder.docker.run_cmd")
    @patch("capsem.builder.docker.detect_runtime")
    def test_arm64_uses_correct_platform_and_volumes(self, mock_detect, mock_run, tmp_path):
        mock_detect.return_value = "docker"
        repo_root = tmp_path / "repo"
        repo_root.mkdir()
        output_dir = tmp_path / "output"

        def side_effect(cmd, **kwargs):
            # Simulate container creating binaries in bind-mounted output
            for b in GUEST_BINARIES:
                (output_dir / b).write_bytes(b"binary content")
            return MagicMock(stdout="")

        mock_run.side_effect = side_effect

        binaries = container_compile_agent(
            "aarch64-unknown-linux-musl", repo_root, output_dir,
        )

        assert len(binaries) == len(GUEST_BINARIES)
        # Single container call (build + cp + file in one step)
        assert mock_run.call_count == 1
        cmd = mock_run.call_args_list[0][0][0]
        assert "docker" in cmd
        assert "--platform" in cmd
        assert "linux/arm64" in cmd
        # Per-arch target volume
        assert "capsem-agent-target-arm64" in str(cmd)
        # Cargo cache volumes
        assert "capsem-cargo-registry" in str(cmd)
        assert "capsem-cargo-git" in str(cmd)

    @patch("capsem.builder.docker.run_cmd")
    @patch("capsem.builder.docker.detect_runtime")
    def test_x86_64_uses_correct_platform_and_volumes(self, mock_detect, mock_run, tmp_path):
        mock_detect.return_value = "docker"
        repo_root = tmp_path / "repo"
        repo_root.mkdir()
        output_dir = tmp_path / "output"

        def side_effect(cmd, **kwargs):
            for b in GUEST_BINARIES:
                (output_dir / b).write_bytes(b"binary content")
            return MagicMock(stdout="")

        mock_run.side_effect = side_effect

        container_compile_agent("x86_64-unknown-linux-musl", repo_root, output_dir)

        cmd = mock_run.call_args_list[0][0][0]
        assert "docker" in cmd
        assert "linux/amd64" in cmd
        assert "capsem-agent-target-x86_64" in str(cmd)

    @patch("capsem.builder.docker.run_cmd")
    @patch("capsem.builder.docker.detect_runtime")
    def test_missing_binary_raises(self, mock_detect, mock_run, tmp_path):
        mock_detect.return_value = "docker"
        repo_root = tmp_path / "repo"
        repo_root.mkdir()
        output_dir = tmp_path / "output"
        # Container runs but doesn't create binaries (simulates build failure)
        mock_run.return_value = MagicMock(stdout="")

        with pytest.raises(RuntimeError, match="Expected binary not found"):
            container_compile_agent(
                "aarch64-unknown-linux-musl", repo_root, output_dir,
            )

    @patch("capsem.builder.docker.run_cmd")
    @patch("capsem.builder.docker.detect_runtime")
    def test_empty_binary_raises(self, mock_detect, mock_run, tmp_path):
        mock_detect.return_value = "docker"
        repo_root = tmp_path / "repo"
        repo_root.mkdir()
        output_dir = tmp_path / "output"

        def side_effect(cmd, **kwargs):
            for b in GUEST_BINARIES:
                (output_dir / b).write_bytes(b"")  # empty
            return MagicMock(stdout="")

        mock_run.side_effect = side_effect

        with pytest.raises(RuntimeError, match="Binary is empty"):
            container_compile_agent(
                "aarch64-unknown-linux-musl", repo_root, output_dir,
            )


class TestCrossCompileAgent:
    """Tests for cross_compile_agent() -- delegates to container on macOS."""

    @patch("capsem.builder.docker.sys")
    @patch("capsem.builder.docker.container_compile_agent")
    def test_delegates_to_container_on_darwin(self, mock_container, mock_sys, tmp_path):
        mock_sys.platform = "darwin"
        mock_container.return_value = []
        cross_compile_agent("aarch64-unknown-linux-musl", tmp_path, tmp_path / "out")
        mock_container.assert_called_once_with(
            "aarch64-unknown-linux-musl", tmp_path, tmp_path / "out",
        )

    @patch("capsem.builder.docker.container_compile_agent")
    @patch("capsem.builder.docker.sys")
    @patch("capsem.builder.docker.run_cmd")
    def test_native_on_linux_skips_container(
        self, mock_run, mock_sys, mock_container, tmp_path,
    ):
        mock_sys.platform = "linux"
        mock_run.return_value = MagicMock(stdout="aarch64-unknown-linux-musl")

        # Create dummy binaries for shutil.copy2
        release_dir = tmp_path / "target" / "aarch64-unknown-linux-musl" / "release"
        release_dir.mkdir(parents=True)
        for b in GUEST_BINARIES:
            (release_dir / b).write_bytes(b"dummy")

        cross_compile_agent("aarch64-unknown-linux-musl", tmp_path, tmp_path / "out")

        # Container path was NOT taken
        mock_container.assert_not_called()
        # Cargo build was called directly
        cargo_calls = [c for c in mock_run.call_args_list if "cargo" in c[0][0]]
        assert len(cargo_calls) == 1
        assert "build" in cargo_calls[0][0][0]
        assert "--target" in cargo_calls[0][0][0]
