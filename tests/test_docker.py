"""Tests for Dockerfile generation and build execution from GuestImageConfig.

TDD: these tests define the expected behavior of docker.py before implementation.
Build execution tests mock run_cmd (single subprocess seam) -- no Docker needed.
"""

import json
import os
import re
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path
from unittest.mock import MagicMock, patch

import pytest

from capsem.builder.config import load_guest_config
from capsem.builder.models import ErofsConfig
from capsem.builder.docker import (
    BUILD_LEDGER_NAME,
    FALLBACK_KERNEL_VERSION,
    GUEST_BINARIES,
    ROOTFS_SCRIPTS,
    _append_build_ledger,
    _directory_tree_hash,
    _file_ledger_entry,
    _rootfs_config_input_record,
    build_all_architectures,
    build_version_script,
    build_image,
    container_compile_agent,
    create_erofs,
    cross_compile_agent,
    detect_runtime,
    docker_build,
    experimental_erofs_build_config,
    export_container_fs,
    extract_tool_versions,
    extract_kernel_assets,
    generate_build_context,
    generate_cyclonedx_obom,
    generate_checksums,
    get_project_version,
    is_ci,
    prepare_build_context,
    render_dockerfile,
    resolve_kernel_version,
    run_cmd,
    sync_container_clock,
)

PROJECT_ROOT = Path(__file__).resolve().parent.parent


def test_run_cmd_surfaces_captured_subprocess_stderr(monkeypatch, capsys):
    """Opaque Docker failures must retain the daemon's actual diagnosis."""
    failure = subprocess.CalledProcessError(
        125,
        ["docker", "run", "missing-image"],
        output="stdout evidence\n",
        stderr="Unable to find image 'missing-image' locally\n",
    )

    def fail(*_args, **_kwargs):
        raise failure

    monkeypatch.setattr(subprocess, "run", fail)

    with pytest.raises(subprocess.CalledProcessError) as raised:
        run_cmd(["docker", "run", "missing-image"], capture=True)

    assert raised.value is failure
    captured = capsys.readouterr()
    assert "stdout evidence" in captured.err
    assert "Unable to find image 'missing-image' locally" in captured.err


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture
def real_config(tmp_path):
    """Load the generated backend image spec used by Docker rendering tests."""
    return _profile_guest_config(tmp_path, "code")


@pytest.fixture
def rendered_arm64(real_config):
    return render_dockerfile("Dockerfile.rootfs.j2", real_config, "arm64")


@pytest.fixture
def rendered_x86(real_config):
    return render_dockerfile("Dockerfile.rootfs.j2", real_config, "x86_64")


def _profile_guest_config(tmp_path: Path, profile_id: str):
    guest = tmp_path / "guest"
    config = guest / "config"
    shutil.copytree(PROJECT_ROOT / "config" / "docker" / "image", config)

    profile_root = PROJECT_ROOT / "config" / "profiles" / profile_id
    profile = tomllib.loads((profile_root / "profile.toml").read_text())

    packages = config / "packages"
    packages.mkdir()
    _write_package_toml(
        packages / "apt.toml",
        "apt",
        "System Packages",
        "apt",
        "apt-get install -y --no-install-recommends",
        _package_lines(profile_root / "apt-packages.txt"),
    )
    _write_package_toml(
        packages / "python.toml",
        "python",
        "Python Packages",
        "uv",
        "uv pip install --system --break-system-packages",
        _package_lines(profile_root / "python-requirements.txt"),
    )
    _write_package_toml(
        packages / "npm.toml",
        "npm",
        "Node Packages",
        "npm",
        "npm install -g --prefix /opt/ai-clis",
        _package_lines(profile_root / "npm-packages.txt"),
    )

    vm = profile["vm"]
    (config / "vm" / "resources.toml").write_text(
        "\n".join(
            [
                "[resources]",
                f"cpu_count = {vm['cpu_count']}",
                f"ram_gb = {vm['ram_gb']}",
                f"scratch_disk_size_gb = {vm['scratch_disk_size_gb']}",
                "log_bodies = false",
                "max_body_capture = 4096",
                "retention_days = 30",
                "max_sessions = 100",
                "min_content_sessions = 25",
                "max_disk_gb = 100",
                "terminated_retention_days = 365",
                "",
            ]
        )
    )

    shutil.copytree(PROJECT_ROOT / "guest" / "artifacts", guest / "artifacts")
    shutil.copytree(profile_root / "root", guest / "profile-root")
    shutil.copy2(profile_root / "build.sh", guest / "profile-build.sh")
    shutil.copy2(profile_root / "tips.txt", guest / "artifacts" / "tips.txt")
    return load_guest_config(guest)


@pytest.fixture
def generated_profile_guest(tmp_path):
    return _profile_guest_config(tmp_path, "code")


def _package_lines(path: Path) -> list[str]:
    return [
        line.strip()
        for line in path.read_text().splitlines()
        if line.strip() and not line.strip().startswith("#")
    ]


def _write_package_toml(
    path: Path,
    key: str,
    name: str,
    manager: str,
    install_cmd: str,
    packages: list[str],
) -> None:
    path.write_text(
        "\n".join(
            [
                f"[{key}]",
                f'name = "{name}"',
                f'manager = "{manager}"',
                f'install_cmd = "{install_cmd}"',
                "packages = [",
                *[f'  "{package}",' for package in packages],
                "]",
                "",
            ]
        )
    )


@pytest.fixture
def rendered_profile_arm64(generated_profile_guest):
    return render_dockerfile("Dockerfile.rootfs.j2", generated_profile_guest, "arm64")


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

    def test_npm_packages_from_package_sets(self, generated_profile_guest, rendered_profile_arm64):
        for pkg in generated_profile_guest.package_sets["npm"].packages:
            assert pkg in rendered_profile_arm64, f"npm package '{pkg}' missing"

    def test_npm_prefix(self, rendered_profile_arm64):
        assert "/opt/ai-clis" in rendered_profile_arm64

    def test_guest_binaries(self, rendered_arm64):
        for binary in GUEST_BINARIES:
            assert f"COPY {binary} " in rendered_arm64
            assert f"chmod 555 /usr/local/bin/{binary}" in rendered_arm64
        assert "COPY capsem-bench-rs /usr/local/bin/capsem-bench-rs" in rendered_arm64

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
        assert "snapshots" in rendered_arm64

    def test_rootfs_includes_all_artifacts(self, rendered_arm64):
        """Every ROOTFS_SCRIPTS entry must appear as a COPY line."""
        for artifact in ROOTFS_SCRIPTS:
            assert f"COPY {artifact}" in rendered_arm64, (
                f"{artifact} missing from rootfs Dockerfile"
            )

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

    def test_env_path_includes_npm_prefix(self, rendered_profile_arm64):
        """Regression: v0.14.18 -- /opt/ai-clis/bin not on PATH, gemini/codex
        returned N/A, build-time validator rejected the rootfs."""
        assert 'ENV PATH="/opt/ai-clis/bin:$PATH"' in rendered_profile_arm64, (
            "Dockerfile.rootfs.j2 must set ENV PATH to include /opt/ai-clis/bin "
            "so version extraction can find npm-installed CLIs"
        )

    def test_env_path_after_npm_install(self, rendered_profile_arm64):
        npm_pos = self._pos(rendered_profile_arm64, "npm install -g --prefix", "npm install")
        path_pos = self._pos(rendered_profile_arm64, 'ENV PATH="/opt/ai-clis/bin', "ENV PATH")
        assert npm_pos < path_pos, "ENV PATH must come after npm install"

    def test_ca_cert_before_certifi_patch(self, rendered_arm64):
        """certifi patch appends our CA to certifi's bundle -- cert must exist first."""
        copy_ca = self._pos(rendered_arm64, "COPY capsem-ca.crt", "COPY CA cert")
        update_ca = self._pos(rendered_arm64, "update-ca-certificates", "update-ca-certificates")
        certifi_patch = self._pos(rendered_arm64, "certifi.where()", "certifi patch")
        assert copy_ca < update_ca < certifi_patch, (
            "Order must be: COPY cert -> update-ca-certificates -> certifi patch"
        )

    def test_node_before_npm_install(self, rendered_profile_arm64):
        """npm install requires node to be installed first."""
        node_pos = self._pos(rendered_profile_arm64, "nvm install", "node install")
        npm_pos = self._pos(rendered_profile_arm64, "npm install -g --prefix", "npm install")
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

    def test_setuid_strip_after_all_installs(self, rendered_profile_arm64):
        """Setuid strip must come after all package installs so no new
        setuid binaries sneak in after the strip."""
        strip_pos = self._pos(rendered_profile_arm64, "-4000", "setuid strip")
        # Must be after npm, python, and guest binary installs
        npm_pos = self._pos(rendered_profile_arm64, "npm install -g --prefix", "npm install")
        assert strip_pos > npm_pos, "setuid strip must come after npm install"
        if "uv pip install --system" in rendered_profile_arm64:
            # Find the LAST uv pip install (python packages, not certifi)
            last_pip = rendered_profile_arm64.rfind("uv pip install --system --break-system-packages")
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

    def test_rootfs_strips_iptables_legacy_frontend(self, rendered_arm64):
        """The guest uses iptables-nft only; strip Debian's legacy frontend."""
        assert "rm -f /usr/sbin/iptables-legacy" in rendered_arm64
        assert "/usr/sbin/ip6tables-legacy" in rendered_arm64


class TestRootfsVersionExtractability:
    """Every tool with a version_command in config must be findable in the
    built image. This class validates that the Dockerfile installs them
    in locations that will be on PATH when extract_tool_versions runs."""

    def test_npm_install_prefix_on_path(self, rendered_profile_arm64):
        assert "/opt/ai-clis/bin" in rendered_profile_arm64

    def test_curl_installed_clis_copied_to_usr_local(self, real_config, rendered_arm64):
        """Curl-installed CLIs write to ~/.local/bin which is tmpfs at runtime.
        The Dockerfile must copy them to /usr/local/bin."""
        has_curl = "curl" in real_config.package_sets
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
        assert "profile_build_script" in ctx
        assert "profile_install_script" not in ctx

    def test_kernel_keys(self, real_config):
        ctx = generate_build_context(
            "Dockerfile.kernel.j2", real_config, "arm64", kernel_version="6.6.127"
        )
        assert "arch" in ctx
        assert "arch_name" in ctx
        assert "kernel_version" in ctx

    def test_rootfs_without_npm_package_set(self, real_config):
        package_sets = {
            key: value for key, value in real_config.package_sets.items() if key != "npm"
        }
        config = real_config.model_copy(update={"package_sets": package_sets})
        ctx = generate_build_context("Dockerfile.rootfs.j2", config, "arm64")
        assert ctx["npm_packages"] == []

    def test_rootfs_npm_packages_can_come_from_profile_package_set(self, generated_profile_guest):
        ctx = generate_build_context("Dockerfile.rootfs.j2", generated_profile_guest, "arm64")
        assert ctx["npm_packages"] == ["@openai/codex", "@google/gemini-cli"]
        rendered = render_dockerfile("Dockerfile.rootfs.j2", generated_profile_guest, "arm64")
        assert "@openai/codex" in rendered
        assert "@google/gemini-cli" in rendered
        assert "profile-build.sh" in rendered
        assert "profile-root/" in rendered

    def test_rootfs_curl_installs(self, real_config):
        ctx = generate_build_context("Dockerfile.rootfs.j2", real_config, "arm64")
        assert ctx["curl_installs"] == []

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
        from capsem.builder.models import GuestImageConfig

        minimal = GuestImageConfig(
            build=real_config.build,
            package_sets={"apt": real_config.package_sets["apt"]},
        )
        result = render_dockerfile("Dockerfile.rootfs.j2", minimal, "arm64")
        assert "FROM --platform=linux/arm64" in result
        # Should not have python install section
        assert "uv pip install --system" not in result or "certifi" in result

    def test_no_npm_package_set(self, real_config):
        """No npm package set means no npm install section."""
        from capsem.builder.models import GuestImageConfig

        minimal = GuestImageConfig(
            build=real_config.build,
            package_sets={"apt": real_config.package_sets["apt"]},
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
    def test_explicit_stable_branch(self, mock_urlopen):
        mock_resp = MagicMock()
        mock_resp.read.return_value = json.dumps({
            "releases": [
                {"version": "7.1-rc6", "moniker": "mainline", "iseol": False},
                {"version": "7.0.11", "moniker": "stable", "iseol": False},
                {"version": "7.0.10", "moniker": "stable", "iseol": False},
                {"version": "6.18.34", "moniker": "longterm", "iseol": False},
            ]
        }).encode()
        mock_resp.__enter__ = lambda s: s
        mock_resp.__exit__ = MagicMock(return_value=False)
        mock_urlopen.return_value = mock_resp
        result = resolve_kernel_version("7.0")
        assert result == "7.0.11"

    @patch("capsem.builder.docker.urllib.request.urlopen")
    def test_auto_stays_on_lts(self, mock_urlopen):
        mock_resp = MagicMock()
        mock_resp.read.return_value = json.dumps({
            "releases": [
                {"version": "7.0.11", "moniker": "stable", "iseol": False},
                {"version": "6.18.34", "moniker": "longterm", "iseol": False},
                {"version": "6.12.92", "moniker": "longterm", "iseol": False},
            ]
        }).encode()
        mock_resp.__enter__ = lambda s: s
        mock_resp.__exit__ = MagicMock(return_value=False)
        mock_urlopen.return_value = mock_resp
        result = resolve_kernel_version("auto")
        assert result == "6.18.34"

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
    def test_syncs_via_bounded_shared_primitive(self, mock_run, mock_sys):
        mock_sys.platform = "darwin"
        mock_sys.executable = sys.executable
        sync_container_clock()
        cmd = mock_run.call_args[0][0]
        assert cmd[0] == sys.executable
        assert cmd[1].endswith("/scripts/sync-container-clock.py")
        assert mock_run.call_args.kwargs["timeout"] == 15

    @patch("capsem.builder.docker.sys")
    @patch("capsem.builder.docker.run_cmd")
    def test_noop_on_linux(self, mock_run, mock_sys):
        mock_sys.platform = "linux"
        sync_container_clock()
        mock_run.assert_not_called()

    @patch("capsem.builder.docker.sys")
    @patch("capsem.builder.docker.run_cmd")
    def test_propagates_errors(self, mock_run, mock_sys):
        mock_sys.platform = "darwin"
        mock_run.side_effect = Exception("VM not running")
        with pytest.raises(Exception, match="VM not running"):
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
# Build execution: rootfs assets
# ---------------------------------------------------------------------------


# ---------------------------------------------------------------------------
# Build execution: version script generation
# ---------------------------------------------------------------------------


class TestBuildVersionScript:
    """build_version_script() assembles a shell script from config."""

    def test_real_config_has_all_sections(self, real_config):
        script = build_version_script(real_config)
        assert '# System' in script
        assert '# Python' not in script

    def test_real_config_has_build_tools(self, real_config):
        script = build_version_script(real_config)
        assert 'node=' in script
        assert 'npm=' in script
        assert 'uv=' in script
        assert 'pip=' in script

    def test_real_config_uses_build_tool_version_commands_only(self, real_config):
        script = build_version_script(real_config)
        assert 'git=' not in script
        assert 'python3=' not in script
        assert 'pytest=' not in script

    def test_empty_config_produces_empty_script(self):
        from capsem.builder.models import BuildConfig, GuestImageConfig
        config = GuestImageConfig(
            build=BuildConfig(architectures={"arm64": real_arch()}),
        )
        script = build_version_script(config)
        assert script == ""

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
    """extract_tool_versions() writes version output from configured commands."""

    @patch("capsem.builder.docker.run_cmd")
    def test_valid_output_passes(self, mock_run, real_config):
        mock_run.return_value = MagicMock(stdout=(
            "# System\n"
            "node=24.1.0\nnpm=10.9.2\nuv=0.7.12\npip=24.0\n"
            "python3=3.11.2\ngit=2.39.5\ngh=2.67.0\ntmux=3.4\ncurl=7.88.1\n"
            "# Python\n"
            "pytest=8.3.4\nnumpy=2.2.3\nrequests=2.32.3\npandas=2.2.3\n"
        ))
        # Should not raise
        extract_tool_versions(
            "docker", "test-image", "linux/arm64",
            Path("/tmp"), real_config,
        )

    @patch("capsem.builder.docker.run_cmd")
    def test_na_values_do_not_raise(self, mock_run, real_config):
        mock_run.return_value = MagicMock(stdout=(
            "# System\n"
            "node=24.1.0\n"
            "# Python\n"
            "pytest=N/A\n"
        ))
        extract_tool_versions(
            "docker", "test-image", "linux/arm64",
            Path("/tmp"), real_config,
        )

    @patch("capsem.builder.docker.run_cmd")
    def test_validate_false_skips_check(self, mock_run, real_config):
        mock_run.return_value = MagicMock(stdout=(
            "# Python\n"
            "pytest=N/A\n"
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
    def test_create_erofs_has_both_options(self, mock_run):
        create_erofs(
            "docker", Path("/tmp/rootfs.tar"), Path("/tmp/rootfs.erofs"),
            "zstd", "65536",
        )
        cmd_str = " ".join(mock_run.call_args[0][0])
        for opt in self.APT_CLOCK_SKEW_OPTIONS:
            assert opt in cmd_str, (
                f"create_erofs() missing apt option '{opt}' -- "
                "erofs builds will fail when container clock drifts"
            )


class TestCreateErofs:
    @patch("capsem.builder.docker.run_cmd")
    def test_zstd_uses_modern_erofs_utils_image(self, mock_run):
        create_erofs(
            "docker", Path("/tmp/rootfs.tar"), Path("/tmp/rootfs.erofs"),
            "zstd", "65536",
        )
        cmd = mock_run.call_args[0][0]
        cmd_str = " ".join(cmd)
        assert "debian:trixie-slim" in cmd
        assert "mkfs.erofs" in cmd_str
        assert "-Enosbcrc" in cmd_str
        assert "-zzstd,level=15" in cmd_str
        assert "-C65536" in cmd_str

    @patch("capsem.builder.docker.run_cmd")
    def test_lz4hc_uses_release_erofs_utils_image(self, mock_run):
        create_erofs(
            "docker", Path("/tmp/rootfs.tar"), Path("/tmp/rootfs.erofs"),
            "lz4hc", "65536", "12",
        )
        cmd = mock_run.call_args[0][0]
        cmd_str = " ".join(cmd)
        assert "debian:bookworm-slim" in cmd
        assert "-Enosbcrc" in cmd_str
        assert "-zlz4hc,level=12" in cmd_str
        assert "-C65536" in cmd_str

    @patch("capsem.builder.docker.run_cmd")
    def test_preserves_output_subdirectory(self, mock_run):
        create_erofs(
            "docker", Path("/tmp/rootfs.tar"), Path("/tmp/out/rootfs.erofs"),
            "zstd", "65536",
        )
        cmd = mock_run.call_args[0][0]
        cmd_str = " ".join(cmd)
        volume_arg = cmd[cmd.index("-v") + 1]
        host_dir, container_dir = volume_arg.split(":", 1)
        assert Path(host_dir).resolve() == Path("/tmp").resolve()
        assert container_dir == "/assets"
        assert "mkdir -p /assets/out" in cmd_str
        assert "tar xf /assets/rootfs.tar -C /rootfs" in cmd_str
        assert " /assets/out/rootfs.erofs /rootfs" in cmd_str

    @patch("capsem.builder.docker.run_cmd")
    def test_chowns_output_to_invoking_user(self, mock_run):
        create_erofs(
            "docker", Path("/tmp/rootfs.tar"), Path("/tmp/out/rootfs.erofs"),
            "lz4hc", None, "12",
        )

        cmd_str = " ".join(mock_run.call_args[0][0])

        assert f"chown {os.getuid()}:{os.getgid()} /assets/out/rootfs.erofs" in cmd_str


class TestBuildLedger:
    def test_file_ledger_entry_uses_blake3_and_relative_path(self, tmp_path):
        data = tmp_path / "context" / "nested" / "file.txt"
        data.parent.mkdir(parents=True)
        data.write_text("ledger")

        entry = _file_ledger_entry(data, base=tmp_path / "context")

        assert entry["path"] == "nested/file.txt"
        assert entry["size"] == len("ledger")
        assert len(entry["blake3"]) == 64

    def test_directory_tree_hash_changes_with_file_contents(self, tmp_path):
        context = tmp_path / "ctx"
        context.mkdir()
        (context / "Dockerfile").write_text("FROM scratch\n")
        first = _directory_tree_hash(context)

        (context / "Dockerfile").write_text("FROM busybox\n")
        second = _directory_tree_hash(context)

        assert first != second

    def test_append_build_ledger_writes_jsonl_records(self, tmp_path):
        arch_output = tmp_path / "assets" / "arm64"
        ledger = _append_build_ledger(arch_output, {
            "stage": "rootfs.erofs",
            "outputs": [{"path": "rootfs.erofs", "size": 4, "blake3": "0" * 64}],
        })

        records = [json.loads(line) for line in ledger.read_text().splitlines()]
        assert ledger.name == BUILD_LEDGER_NAME
        assert records[0]["schema"] == "capsem.build_ledger.v1"
        assert records[0]["stage"] == "rootfs.erofs"
        assert records[0]["outputs"][0]["path"] == "rootfs.erofs"

    def test_rootfs_config_input_record_tracks_declared_inputs_not_installed_state(
        self, generated_profile_guest
    ):
        record = _rootfs_config_input_record(generated_profile_guest, "arm64")

        assert record["stage"] == "rootfs.config_inputs"
        assert record["arch"] == "arm64"
        assert "curl" in record["package_inputs"]["apt"]["packages"]
        assert "zstd" in record["package_inputs"]["apt"]["packages"]
        assert "pytest" in record["package_inputs"]["python"]["packages"]
        assert "openai" in record["package_inputs"]["python"]["packages"]
        assert record["package_inputs"]["npm"]["packages"] == [
            "@openai/codex",
            "@google/gemini-cli",
        ]
        assert record["package_inputs"]["python"]["install_cmd"] == (
            "uv pip install --system --break-system-packages"
        )
        assert record["profile_inputs"]["root_seed"]["enabled"] is True
        assert record["profile_inputs"]["build_script"]["enabled"] is True
        assert record["erofs"] == {
            "enabled": True,
            "compression": "lz4hc",
            "compression_level": 12,
            "cluster_size": None,
        }
        assert "installed_packages" not in record
        assert "installed_versions" not in record

    @patch("capsem.builder.docker.run_cmd")
    def test_generate_cyclonedx_obom_extracts_rootfs_and_runs_cdxgen(self, mock_run, tmp_path, monkeypatch):
        repo_root = tmp_path
        (repo_root / "target" / "tmp").mkdir(parents=True)
        rootfs_tar = tmp_path / "rootfs.tar"
        rootfs_tar.write_bytes(b"tar")
        output = tmp_path / "assets" / "arm64" / "obom.cdx.json"
        monkeypatch.setenv("CAPSEM_CDXGEN_CMD", "cdxgen")

        def fake_run(cmd, **_kwargs):
            if cmd[0] == "cdxgen":
                output.write_text(json.dumps({
                    "bomFormat": "CycloneDX",
                    "metadata": {
                        "tools": {
                            "components": [
                                {"name": "cdxgen", "version": "11.0.0"}
                            ]
                        }
                    },
                    "components": [],
                }))
            return MagicMock(stdout="")

        mock_run.side_effect = fake_run

        result = generate_cyclonedx_obom(rootfs_tar, output, repo_root=repo_root)

        assert result == output
        tar_cmd = mock_run.call_args_list[0][0][0]
        assert tar_cmd[0] == "tar"
        assert "--exclude=dev/*" in tar_cmd
        assert "-xf" in tar_cmd
        assert str(rootfs_tar) in tar_cmd
        cdxgen_cmd = mock_run.call_args_list[1][0][0]
        assert cdxgen_cmd[:4] == ["cdxgen", "-t", "os", "-o"]
        assert cdxgen_cmd[4] == str(output)

    @patch("capsem.builder.docker.remove_image")
    @patch("capsem.builder.docker.extract_software_inventory")
    @patch("capsem.builder.docker.extract_tool_versions")
    @patch("capsem.builder.docker.generate_cyclonedx_obom")
    @patch("capsem.builder.docker.create_erofs")
    @patch("capsem.builder.docker.export_container_fs")
    @patch("capsem.builder.docker.docker_build")
    @patch("capsem.builder.docker.cross_compile_agent")
    @patch("capsem.builder.docker.sync_container_clock")
    @patch("capsem.builder.docker.detect_runtime")
    def test_rootfs_build_records_export_erofs_and_versions(
        self,
        mock_runtime,
        _mock_sync,
        mock_cross_compile,
        _mock_docker_build,
        mock_export,
        mock_create_erofs,
        mock_generate_obom,
        mock_extract_versions,
        mock_extract_inventory,
        _mock_remove,
        real_config,
        tmp_path,
    ):
        mock_runtime.return_value = "docker"

        def fake_cross_compile(_rust_target, _repo_root, context_dir):
            copied = []
            for binary in GUEST_BINARIES:
                path = context_dir / binary
                path.write_text(binary)
                copied.append(path)
            return copied

        def fake_export(_runtime, _tag, _platform, output_tar):
            output_tar.write_bytes(b"rootfs tar")

        def fake_erofs(_runtime, _tar_path, output_path, *_args):
            output_path.write_bytes(b"erofs")

        def fake_obom(_tar_path, output_path, **_kwargs):
            output_path.write_text(json.dumps({
                "bomFormat": "CycloneDX",
                "metadata": {
                    "tools": {
                        "components": [
                            {"name": "cdxgen", "version": "11.0.0"}
                        ]
                    }
                },
                "components": [],
            }))

        def fake_versions(_runtime, _tag, _platform, output_dir, _config):
            (output_dir / "tool-versions.txt").write_text("codex=1.0.0\n")

        def fake_inventory(_runtime, _tag, _platform, _arch_name, output_dir):
            path = output_dir / "software-inventory.json"
            path.write_text(
                json.dumps({
                    "schema": "capsem.profile_software_inventory.v1",
                    "architecture": "arm64",
                    "packages": [],
                })
            )
            return path

        mock_cross_compile.side_effect = fake_cross_compile
        mock_export.side_effect = fake_export
        mock_create_erofs.side_effect = fake_erofs
        mock_generate_obom.side_effect = fake_obom
        mock_extract_versions.side_effect = fake_versions
        mock_extract_inventory.side_effect = fake_inventory

        build_image(
            real_config,
            "arm64",
            template="rootfs",
            output_dir=tmp_path,
            repo_root=PROJECT_ROOT,
        )

        records = [
            json.loads(line)
            for line in (tmp_path / "arm64" / BUILD_LEDGER_NAME).read_text().splitlines()
        ]
        assert [record["stage"] for record in records] == [
            "rootfs.config_inputs",
            "rootfs.software_inventory",
            "rootfs.export",
            "rootfs.erofs",
            "rootfs.obom",
            "rootfs.tool_versions",
        ]
        config_record = records[0]
        assert config_record["package_inputs"]["apt"]["packages"]
        assert config_record["profile_inputs"]["root_seed"]["enabled"] is True
        assert "installed_packages" not in config_record
        inventory_record = records[1]
        assert inventory_record["outputs"][0]["path"] == "software-inventory.json"
        erofs_record = records[3]
        assert erofs_record["erofs"] == {
            "compression": "lz4hc",
            "compression_level": "12",
            "cluster_size": None,
            "utils_image": "debian:bookworm-slim",
        }
        assert erofs_record["outputs"][0]["path"] == "rootfs.erofs"
        assert erofs_record["inputs"]["build_context"]["hash"]
        obom_record = records[4]
        assert obom_record["generator"] == "cdxgen"
        assert obom_record["outputs"][0]["path"] == "obom.cdx.json"

    @patch("capsem.builder.docker.remove_image")
    @patch("capsem.builder.docker.extract_kernel_assets")
    @patch("capsem.builder.docker.docker_build")
    @patch("capsem.builder.docker.sync_container_clock")
    @patch("capsem.builder.docker.detect_runtime")
    def test_kernel_build_records_assets(
        self,
        mock_runtime,
        _mock_sync,
        _mock_docker_build,
        mock_extract,
        _mock_remove,
        real_config,
        tmp_path,
    ):
        mock_runtime.return_value = "docker"

        def fake_extract(_runtime, _tag, _platform, output_dir):
            vmlinuz = output_dir / "vmlinuz"
            initrd = output_dir / "initrd.img"
            vmlinuz.write_bytes(b"kernel")
            initrd.write_bytes(b"initrd")
            return vmlinuz, initrd

        mock_extract.side_effect = fake_extract

        build_image(
            real_config,
            "arm64",
            template="kernel",
            output_dir=tmp_path,
            kernel_version="7.0.11",
            repo_root=PROJECT_ROOT,
        )

        records = [
            json.loads(line)
            for line in (tmp_path / "arm64" / BUILD_LEDGER_NAME).read_text().splitlines()
        ]
        assert len(records) == 1
        assert records[0]["stage"] == "kernel.assets"
        assert records[0]["kernel_version"] == "7.0.11"
        assert {entry["path"] for entry in records[0]["outputs"]} == {
            "vmlinuz",
            "initrd.img",
        }

class TestErofsConfig:
    def test_config_defaults_enable_release_lz4hc(self):
        assert experimental_erofs_build_config({}, ErofsConfig()) == (
            True, "lz4hc", None, "12",
        )

    def test_env_cannot_disable_release_erofs(self):
        with pytest.raises(ValueError, match="EROFS build cannot be disabled"):
            experimental_erofs_build_config(
                {"CAPSEM_BUILD_EXPERIMENTAL_EROFS": "0"},
                ErofsConfig(),
            )

    def test_env_config_parses_enabled_zstd(self):
        assert experimental_erofs_build_config({
            "CAPSEM_BUILD_EXPERIMENTAL_EROFS": "1",
            "CAPSEM_BUILD_EROFS_COMPRESSION": "zstd",
            "CAPSEM_BUILD_EROFS_CLUSTER_SIZE": "65536",
        }) == (True, "zstd", "65536", "15")

    def test_env_config_rejects_unknown_compression(self):
        with pytest.raises(ValueError, match="CAPSEM_BUILD_EROFS_COMPRESSION"):
            experimental_erofs_build_config({
                "CAPSEM_BUILD_EXPERIMENTAL_EROFS": "1",
                "CAPSEM_BUILD_EROFS_COMPRESSION": "brotli",
            })

    def test_env_config_rejects_zstd_level_outside_range(self):
        with pytest.raises(ValueError, match="0..22"):
            experimental_erofs_build_config({
                "CAPSEM_BUILD_EXPERIMENTAL_EROFS": "1",
                "CAPSEM_BUILD_EROFS_COMPRESSION": "zstd",
                "CAPSEM_BUILD_EROFS_COMPRESSION_LEVEL": "23",
            })

    def test_env_config_rejects_lz4_level(self):
        with pytest.raises(ValueError, match="not valid for lz4"):
            experimental_erofs_build_config({
                "CAPSEM_BUILD_EXPERIMENTAL_EROFS": "1",
                "CAPSEM_BUILD_EROFS_COMPRESSION": "lz4",
                "CAPSEM_BUILD_EROFS_COMPRESSION_LEVEL": "1",
            })


class TestKernelConfig:
    def test_real_config_pins_stable_kernel_branch(self, real_config):
        assert real_config.build.architectures["arm64"].kernel_branch == "7.0"
        assert real_config.build.architectures["x86_64"].kernel_branch == "7.0"

    def test_real_config_defaults_erofs_lz4hc_level_12(self, real_config):
        assert real_config.build.erofs.enabled is True
        assert real_config.build.erofs.compression.value == "lz4hc"
        assert real_config.build.erofs.compression_level == 12

    @pytest.mark.parametrize("name", ["defconfig.arm64", "defconfig.x86_64"])
    def test_erofs_zstd_enabled(self, name):
        content = (PROJECT_ROOT / "config" / "docker" / "image" / "kernel" / name).read_text()
        assert "CONFIG_EROFS_FS=y" in content
        assert "CONFIG_EROFS_FS_ZIP=y" in content
        assert "CONFIG_EROFS_FS_ZIP_ZSTD=y" in content

    @pytest.mark.parametrize("name", ["defconfig.arm64", "defconfig.x86_64"])
    def test_kvm_virtio_mmio_transport_enabled(self, name):
        content = (PROJECT_ROOT / "config" / "docker" / "image" / "kernel" / name).read_text()
        assert "CONFIG_VIRTIO_MMIO=y" in content
        if name == "defconfig.x86_64":
            assert "CONFIG_VIRTIO_MMIO_CMDLINE_DEVICES=y" in content

    @pytest.mark.parametrize("name", ["defconfig.arm64", "defconfig.x86_64"])
    def test_iptables_nft_nat_redirect_enabled(self, name):
        content = (PROJECT_ROOT / "config" / "docker" / "image" / "kernel" / name).read_text()
        required = [
            "CONFIG_NETFILTER=y",
            "CONFIG_NF_TABLES=y",
            "CONFIG_NF_TABLES_IPV4=y",
            "CONFIG_NFT_NAT=y",
            "CONFIG_NFT_REDIR=y",
            "CONFIG_NETFILTER_XTABLES=y",
            "CONFIG_NFT_COMPAT=y",
            "CONFIG_NETFILTER_XT_TARGET_REDIRECT=y",
            "CONFIG_NF_NAT_REDIRECT=y",
        ]
        for symbol in required:
            assert symbol in content
        forbidden = [
            "CONFIG_NETFILTER_XTABLES_LEGACY=y",
            "CONFIG_IP_NF_IPTABLES_LEGACY=y",
            "CONFIG_IP_NF_IPTABLES=y",
            "CONFIG_IP_NF_NAT=y",
            "CONFIG_IP_NF_TARGET_REDIRECT=y",
        ]
        for symbol in forbidden:
            assert symbol not in content

    def test_init_mounts_erofs_by_default(self):
        content = (PROJECT_ROOT / "guest" / "artifacts" / "capsem-init").read_text()
        assert "ROOTFS_TYPE=erofs" in content
        assert "ROOTFS_LABEL=erofs" in content
        assert "capsem.rootfs=erofs-dax" in content
        assert "ROOTFS_MOUNT_OPTS=ro,dax" in content
        assert 'mount -t "$ROOTFS_TYPE" -o "$ROOTFS_MOUNT_OPTS" /dev/vda /mnt/a' in content
        assert 'boot_mark "$ROOTFS_LABEL"' in content
        assert "FATAL: cannot mount /dev/vda" in content

    def test_init_uses_iptables_nft_only(self):
        content = (PROJECT_ROOT / "guest" / "artifacts" / "capsem-init").read_text()
        assert "iptables-nft" in content
        assert "iptables-legacy" not in content
        assert 'IPTABLES=iptables-nft' in content
        assert "iptables_add()" in content
        assert "FATAL: iptables-nft failed" in content
        assert 'chroot /newroot "$IPTABLES" -t nat -A' not in content


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
        assert (context_dir / "snapshots").is_file()
        assert (context_dir / "capsem_bench").is_dir()
        assert (context_dir / "capsem_bench" / "__main__.py").is_file()
        # Snapshot CLI must be in rootfs context
        assert (context_dir / "snapshots").is_file()

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

    def test_rootfs_context_copies_profile_root_and_build_script(
        self, generated_profile_guest, tmp_path
    ):
        context_dir = tmp_path / "ctx"
        context_dir.mkdir()
        prepare_build_context(
            generated_profile_guest,
            "arm64",
            "Dockerfile.rootfs.j2",
            context_dir,
            PROJECT_ROOT,
        )
        assert (context_dir / "profile-build.sh").is_file()
        assert (context_dir / "profile-root/root/.antigravity/config.json").is_file()
        assert (context_dir / "profile-root/root/.gemini/config/config.json").is_file()
        assert (context_dir / "profile-root/root/.gemini/antigravity-cli/settings.json").is_file()
        assert (context_dir / "profile-root/root/.codex/config.toml").is_file()
        assert "Credentials are brokered by Capsem" in (context_dir / "tips.txt").read_text()

        forbidden_fragments = (
            "127.0.0.1:11434",
            "localhost:11434",
            "CAPSEM_MOCK_SERVER",
            '"provider": "ollama"',
            '"baseUrl": "http://127.0.0.1:11434"',
        )
        leaked = []
        for payload in sorted((context_dir / "profile-root").rglob("*")):
            if not payload.is_file():
                continue
            text = payload.read_text(errors="ignore")
            for fragment in forbidden_fragments:
                if fragment in text:
                    leaked.append(
                        f"{payload.relative_to(context_dir / 'profile-root')}: {fragment}"
                    )
        assert leaked == []

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
        (arm64 / "rootfs.erofs").write_bytes(b"rootfs")
        generate_checksums(tmp_path, "0.13.0")
        # B3SUMS was written
        assert (tmp_path / "B3SUMS").exists()
        # manifest.json was written (v2 format: orthogonal assets vs binaries).
        manifest = json.loads((tmp_path / "manifest.json").read_text())
        assert manifest["format"] == 2
        assert manifest["refresh_policy"] == "24h"
        assert manifest["binaries"]["current"] == "0.13.0"
        assert "0.13.0" in manifest["binaries"]["releases"]
        asset_version = manifest["assets"]["current"]
        assert asset_version in manifest["assets"]["releases"]

    def test_manifest_reuses_release_for_identical_assets(self, tmp_path):
        arm64 = tmp_path / "arm64"
        arm64.mkdir()
        (arm64 / "vmlinuz").write_bytes(b"kernel")
        (arm64 / "initrd.img").write_bytes(b"initrd")
        (arm64 / "rootfs.erofs").write_bytes(b"rootfs")

        generate_checksums(tmp_path, "0.13.0")
        first = json.loads((tmp_path / "manifest.json").read_text())
        generate_checksums(tmp_path, "0.13.0")
        second = json.loads((tmp_path / "manifest.json").read_text())

        assert second["assets"]["current"] == first["assets"]["current"]

    def test_manifest_increments_release_for_changed_assets(self, tmp_path):
        arm64 = tmp_path / "arm64"
        arm64.mkdir()
        (arm64 / "vmlinuz").write_bytes(b"kernel")
        (arm64 / "initrd.img").write_bytes(b"initrd")
        (arm64 / "rootfs.erofs").write_bytes(b"rootfs")

        generate_checksums(tmp_path, "0.13.0")
        first = json.loads((tmp_path / "manifest.json").read_text())
        (arm64 / "initrd.img").write_bytes(b"changed-initrd")
        generate_checksums(tmp_path, "0.13.0")
        second = json.loads((tmp_path / "manifest.json").read_text())

        assert first["assets"]["current"].endswith(".1")
        assert second["assets"]["current"].endswith(".2")

    def test_manifest_per_arch_structure(self, tmp_path):
        """Per-arch layout produces releases[v].arches[arch][filename]={hash,size}."""
        arm64 = tmp_path / "arm64"
        arm64.mkdir()
        (arm64 / "vmlinuz").write_bytes(b"kernel")
        (arm64 / "initrd.img").write_bytes(b"initrd")
        (arm64 / "rootfs.erofs").write_bytes(b"rootfs")
        generate_checksums(tmp_path, "0.13.0")
        manifest = json.loads((tmp_path / "manifest.json").read_text())
        asset_version = manifest["assets"]["current"]
        release = manifest["assets"]["releases"][asset_version]
        assert "arm64" in release["arches"], "per-arch key 'arm64' missing"
        arm64_entries = release["arches"]["arm64"]
        assert set(arm64_entries) == {"vmlinuz", "initrd.img", "rootfs.erofs"}
        for filename, entry in arm64_entries.items():
            assert "/" not in filename
            assert len(entry["hash"]) == 64  # blake3 hex digest
            assert entry["size"] > 0

    def test_manifest_includes_obom_when_rootfs_build_emits_it(self, tmp_path):
        """CycloneDX OBOM is pinned as a profile asset, not replaced by build-ledger."""
        arm64 = tmp_path / "arm64"
        arm64.mkdir()
        (arm64 / "vmlinuz").write_bytes(b"kernel")
        (arm64 / "initrd.img").write_bytes(b"initrd")
        (arm64 / "rootfs.erofs").write_bytes(b"rootfs")
        (arm64 / "obom.cdx.json").write_text(json.dumps({
            "bomFormat": "CycloneDX",
            "metadata": {
                "tools": {
                    "components": [
                        {"name": "cdxgen", "version": "11.0.0"}
                    ]
                }
            },
        }))

        generate_checksums(tmp_path, "0.13.0")

        manifest = json.loads((tmp_path / "manifest.json").read_text())
        asset_version = manifest["assets"]["current"]
        arm64_entries = manifest["assets"]["releases"][asset_version]["arches"]["arm64"]
        assert "obom.cdx.json" in arm64_entries
        assert "build-ledger.log" not in arm64_entries

    def test_manifest_regen_restores_canonical_software_inventory_from_alias(self, tmp_path):
        arm64 = tmp_path / "arm64"
        arm64.mkdir()
        (arm64 / "vmlinuz").write_bytes(b"kernel")
        (arm64 / "initrd.img").write_bytes(b"initrd")
        (arm64 / "rootfs.erofs").write_bytes(b"rootfs")
        (arm64 / "software-inventory.json").write_text(
            json.dumps({"schema": "capsem.profile_software_inventory.v1", "packages": []})
        )
        generate_checksums(tmp_path, "0.13.0")
        manifest = json.loads((tmp_path / "manifest.json").read_text())
        asset_version = manifest["assets"]["current"]
        digest = manifest["assets"]["releases"][asset_version]["arches"]["arm64"][
            "software-inventory.json"
        ]["hash"]
        alias = arm64 / f"software-inventory-{digest[:16]}.json"
        shutil.copy2(arm64 / "software-inventory.json", alias)
        (arm64 / "software-inventory.json").unlink()

        generate_checksums(tmp_path, "0.13.0")

        assert (arm64 / "software-inventory.json").read_bytes() == alias.read_bytes()
        b3sums = (tmp_path / "B3SUMS").read_text()
        assert "arm64/software-inventory.json" in b3sums

    def test_manifest_flat_fallback(self, tmp_path):
        """Flat layout (no arch subdirs) still populates an arches entry."""
        (tmp_path / "vmlinuz").write_bytes(b"kernel")
        (tmp_path / "initrd.img").write_bytes(b"initrd")
        (tmp_path / "rootfs.erofs").write_bytes(b"rootfs")
        generate_checksums(tmp_path, "0.13.0")
        manifest = json.loads((tmp_path / "manifest.json").read_text())
        asset_version = manifest["assets"]["current"]
        arches = manifest["assets"]["releases"][asset_version]["arches"]
        # Flat layout stores entries under a single "unknown" arch key.
        assert len(arches) == 1
        (only_arch,) = arches
        filenames = set(arches[only_arch])
        assert filenames == {"vmlinuz", "initrd.img", "rootfs.erofs"}

    def test_manifest_multi_arch(self, tmp_path):
        """Both arm64 and x86_64 subdirs produce both arch keys."""
        for arch in ("arm64", "x86_64"):
            d = tmp_path / arch
            d.mkdir()
            (d / "vmlinuz").write_bytes(f"kernel-{arch}".encode())
            (d / "initrd.img").write_bytes(f"initrd-{arch}".encode())
            (d / "rootfs.erofs").write_bytes(f"rootfs-{arch}".encode())
        generate_checksums(tmp_path, "0.13.0")
        manifest = json.loads((tmp_path / "manifest.json").read_text())
        asset_version = manifest["assets"]["current"]
        arches = manifest["assets"]["releases"][asset_version]["arches"]
        assert set(arches) == {"arm64", "x86_64"}
        for arch in ("arm64", "x86_64"):
            assert set(arches[arch]) == {"vmlinuz", "initrd.img", "rootfs.erofs"}
        arm_hashes = {entry["hash"] for entry in arches["arm64"].values()}
        x86_hashes = {entry["hash"] for entry in arches["x86_64"].values()}
        assert arm_hashes.isdisjoint(x86_hashes)

    def test_manifest_prefers_erofs_when_both_rootfs_formats_exist(self, tmp_path):
        """EROFS is the canonical rootfs when both modern and legacy files exist."""
        arm64 = tmp_path / "arm64"
        arm64.mkdir()
        (arm64 / "vmlinuz").write_bytes(b"kernel")
        (arm64 / "initrd.img").write_bytes(b"initrd")
        (arm64 / "rootfs.erofs").write_bytes(b"erofs")
        (arm64 / "rootfs.squashfs").write_bytes(b"squashfs")

        generate_checksums(tmp_path, "0.13.0")

        b3sums = (tmp_path / "B3SUMS").read_text()
        assert "arm64/rootfs.erofs" in b3sums
        assert "arm64/rootfs.squashfs" not in b3sums

        manifest = json.loads((tmp_path / "manifest.json").read_text())
        asset_version = manifest["assets"]["current"]
        entries = manifest["assets"]["releases"][asset_version]["arches"]["arm64"]
        assert "rootfs.erofs" in entries
        assert "rootfs.squashfs" not in entries

    def test_manifest_rejects_squashfs_when_erofs_is_absent(self, tmp_path):
        """A squashfs-only asset directory must not mint a release manifest."""
        arm64 = tmp_path / "arm64"
        arm64.mkdir()
        (arm64 / "vmlinuz").write_bytes(b"kernel")
        (arm64 / "initrd.img").write_bytes(b"initrd")
        (arm64 / "rootfs.squashfs").write_bytes(b"rootfs")

        with pytest.raises(FileNotFoundError, match="rootfs.erofs"):
            generate_checksums(tmp_path, "0.13.0")

    def test_manifest_rejects_rootfs_only_arch(self, tmp_path):
        """A rootfs-only partial build must not clobber a bootable manifest."""
        arm64 = tmp_path / "arm64"
        arm64.mkdir()
        (arm64 / "rootfs.erofs").write_bytes(b"rootfs")

        with pytest.raises(FileNotFoundError, match="vmlinuz"):
            generate_checksums(tmp_path, "0.13.0")

    def test_manifest_rejects_kernel_only_arch(self, tmp_path):
        """A kernel-only partial build must not mint a rootfs-less manifest."""
        arm64 = tmp_path / "arm64"
        arm64.mkdir()
        (arm64 / "vmlinuz").write_bytes(b"kernel")
        (arm64 / "initrd.img").write_bytes(b"initrd")

        with pytest.raises(FileNotFoundError, match="rootfs.erofs"):
            generate_checksums(tmp_path, "0.13.0")


class TestBuildAllArchitectures:
    def test_kernel_only_template_does_not_generate_full_asset_manifest(self, real_config, tmp_path):
        """Kernel-only CI primitive must not require rootfs.erofs before rootfs build runs."""
        with (
            patch("capsem.builder.docker.build_image") as build_image_mock,
            patch("capsem.builder.docker.detect_runtime", return_value="docker"),
            patch("capsem.builder.docker.run_cmd"),
            patch("capsem.builder.docker.generate_checksums") as checksums,
            patch("capsem.builder.docker.get_project_version", return_value="0.13.0"),
        ):
            build_all_architectures(
                real_config,
                template="kernel",
                output_dir=tmp_path,
                repo_root=PROJECT_ROOT,
            )

        assert build_image_mock.call_count == len(real_config.build.architectures)
        checksums.assert_not_called()

    def test_rootfs_template_generates_full_asset_manifest(self, real_config, tmp_path):
        with (
            patch("capsem.builder.docker.build_image"),
            patch("capsem.builder.docker.detect_runtime", return_value="docker"),
            patch("capsem.builder.docker.run_cmd"),
            patch("capsem.builder.docker.generate_checksums") as checksums,
            patch("capsem.builder.docker.get_project_version", return_value="0.13.0"),
        ):
            build_all_architectures(
                real_config,
                template="rootfs",
                output_dir=tmp_path,
                repo_root=PROJECT_ROOT,
            )

        checksums.assert_called_once_with(tmp_path, "0.13.0")


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
        # Two calls: `docker image inspect` pre-check, then the actual `docker run`.
        assert mock_run.call_count == 2
        run_cmd = mock_run.call_args_list[-1][0][0]
        assert "docker" in run_cmd
        assert "--platform" in run_cmd
        assert "linux/arm64" in run_cmd
        # Per-arch target volume
        assert "capsem-agent-target-arm64" in str(run_cmd)
        # Cargo cache volumes
        assert "capsem-cargo-registry" in str(run_cmd)
        assert "capsem-cargo-git" in str(run_cmd)

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

        # Last call is the actual `docker run`; prior calls may include
        # `docker image inspect` preflight.
        cmd = mock_run.call_args_list[-1][0][0]
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


class TestContainerCompileAgentShellScript:
    """Verify the shell script passed to bash -c in container builds.

    The script symlinks most of /src into /build but must exclude certain
    dirs (target, Cargo.lock, crates) and handle crates via cp -r instead.
    Regression: symlinked crates broke workspace resolution inside the container.
    """

    def _extract_shell_script(self, mock_run, tmp_path):
        """Run container_compile_agent and return the bash -c script string."""
        mock_run.side_effect = lambda cmd, **kw: [
            (tmp_path / "output" / b).write_bytes(b"elf")
            for b in GUEST_BINARIES
        ] or MagicMock(stdout="")
        container_compile_agent(
            "aarch64-unknown-linux-musl",
            tmp_path / "repo",
            tmp_path / "output",
        )
        # Find the `docker run` invocation (skip any `docker image inspect` pre-check).
        for call in mock_run.call_args_list:
            cmd = call[0][0]
            if "run" in cmd:
                return cmd[-1]  # bash -c argument is the last element
        raise AssertionError("no `docker run` call recorded")

    @patch("capsem.builder.docker.run_cmd")
    @patch("capsem.builder.docker.detect_runtime", return_value="docker")
    def test_shell_excludes_crates_from_symlinks(self, _det, mock_run, tmp_path):
        (tmp_path / "repo").mkdir()
        script = self._extract_shell_script(mock_run, tmp_path)
        assert '[ "$b" != crates ]' in script

    @patch("capsem.builder.docker.run_cmd")
    @patch("capsem.builder.docker.detect_runtime", return_value="docker")
    def test_shell_copies_crates_with_cp(self, _det, mock_run, tmp_path):
        (tmp_path / "repo").mkdir()
        script = self._extract_shell_script(mock_run, tmp_path)
        assert "cp -r /src/crates /build/crates" in script

    @patch("capsem.builder.docker.run_cmd")
    @patch("capsem.builder.docker.detect_runtime", return_value="docker")
    def test_shell_excludes_target_from_symlinks(self, _det, mock_run, tmp_path):
        (tmp_path / "repo").mkdir()
        script = self._extract_shell_script(mock_run, tmp_path)
        assert '[ "$b" != target ]' in script

    @patch("capsem.builder.docker.run_cmd")
    @patch("capsem.builder.docker.detect_runtime", return_value="docker")
    def test_shell_excludes_cargo_lock_from_symlinks(self, _det, mock_run, tmp_path):
        (tmp_path / "repo").mkdir()
        script = self._extract_shell_script(mock_run, tmp_path)
        assert '[ "$b" != Cargo.lock ]' in script

    @patch("capsem.builder.docker.run_cmd")
    @patch("capsem.builder.docker.detect_runtime", return_value="docker")
    def test_shell_copies_all_binaries_to_output(self, _det, mock_run, tmp_path):
        (tmp_path / "repo").mkdir()
        script = self._extract_shell_script(mock_run, tmp_path)
        for binary in GUEST_BINARIES:
            assert f"cp target/aarch64-unknown-linux-musl/release/{binary} /output/{binary}" in script

    @patch("capsem.builder.docker.run_cmd")
    @patch("capsem.builder.docker.detect_runtime", return_value="docker")
    def test_shell_lists_all_binaries_for_verification(self, _det, mock_run, tmp_path):
        (tmp_path / "repo").mkdir()
        script = self._extract_shell_script(mock_run, tmp_path)
        for binary in GUEST_BINARIES:
            assert f"ls -l /output/{binary}" in script

    @patch("capsem.builder.docker.run_cmd")
    @patch("capsem.builder.docker.detect_runtime", return_value="docker")
    def test_shell_builds_capsem_agent_package(self, _det, mock_run, tmp_path):
        (tmp_path / "repo").mkdir()
        script = self._extract_shell_script(mock_run, tmp_path)
        assert "cargo build --release --target aarch64-unknown-linux-musl -p capsem-agent" in script


class TestPrepareBuildContextArtifacts:
    """Tests for rootfs artifact handling in prepare_build_context.

    Verifies that ROOTFS_SCRIPTS entries are copied when present and
    silently skipped when missing, and that directory artifacts (diagnostics,
    capsem_bench) behave the same way.
    """

    @pytest.fixture
    def fake_repo(self, tmp_path):
        """Build a minimal fake repo root with all rootfs build context files."""
        repo = tmp_path / "repo"
        artifacts = repo / "guest" / "artifacts"
        artifacts.mkdir(parents=True)
        security_keys = repo / "security" / "keys"
        security_keys.mkdir(parents=True)
        (security_keys / "capsem-ca.crt").write_text("fake cert")
        for name in ("capsem-bashrc", "banner.txt", "tips.txt"):
            (artifacts / name).write_text(f"content of {name}")
        for name in ROOTFS_SCRIPTS:
            (artifacts / name).write_text(f"content of {name}")
        diag = artifacts / "diagnostics"
        diag.mkdir()
        (diag / "conftest.py").write_text("# diag conf")
        (diag / "test_sandbox.py").write_text("# diag test")
        bench = artifacts / "capsem_bench"
        bench.mkdir()
        (bench / "__main__.py").write_text("# bench main")
        (bench / "disk.py").write_text("# disk bench")
        return repo

    def fake_guest_config(self, real_config, fake_repo):
        """Point the backend image spec at the fake guest workspace."""
        return real_config.model_copy(
            update={"guest_dir_path": str(fake_repo / "guest")}
        )

    def test_missing_rootfs_artifact_silently_skipped(self, real_config, fake_repo, tmp_path):
        # Remove one ROOTFS_SCRIPT from fake repo
        (fake_repo / "guest" / "artifacts" / "snapshots").unlink()
        ctx = tmp_path / "ctx"
        ctx.mkdir()
        config = self.fake_guest_config(real_config, fake_repo)
        prepare_build_context(config, "arm64", "Dockerfile.rootfs.j2", ctx, fake_repo)
        assert not (ctx / "snapshots").exists()
        # Other artifacts still copied
        assert (ctx / "capsem-doctor").is_file()
        assert (ctx / "capsem-bench").is_file()

    def test_all_rootfs_artifacts_copied_when_present(self, real_config, fake_repo, tmp_path):
        ctx = tmp_path / "ctx"
        ctx.mkdir()
        config = self.fake_guest_config(real_config, fake_repo)
        prepare_build_context(config, "arm64", "Dockerfile.rootfs.j2", ctx, fake_repo)
        for name in ROOTFS_SCRIPTS:
            assert (ctx / name).is_file(), f"{name} not copied to build context"

    def test_missing_diagnostics_dir_no_crash(self, real_config, fake_repo, tmp_path):
        shutil.rmtree(fake_repo / "guest" / "artifacts" / "diagnostics")
        ctx = tmp_path / "ctx"
        ctx.mkdir()
        config = self.fake_guest_config(real_config, fake_repo)
        prepare_build_context(config, "arm64", "Dockerfile.rootfs.j2", ctx, fake_repo)
        assert not (ctx / "diagnostics").exists()

    def test_missing_bench_pkg_dir_no_crash(self, real_config, fake_repo, tmp_path):
        shutil.rmtree(fake_repo / "guest" / "artifacts" / "capsem_bench")
        ctx = tmp_path / "ctx"
        ctx.mkdir()
        config = self.fake_guest_config(real_config, fake_repo)
        prepare_build_context(config, "arm64", "Dockerfile.rootfs.j2", ctx, fake_repo)
        assert not (ctx / "capsem_bench").exists()


class TestRootfsArtifactConstants:
    """Consistency checks for ROOTFS_SCRIPTS and ROOTFS_SCRIPT_DIRS."""

    def test_rootfs_artifacts_no_duplicates(self):
        assert len(ROOTFS_SCRIPTS) == len(set(ROOTFS_SCRIPTS))

    def test_rootfs_artifact_dirs_no_duplicates(self):
        from capsem.builder.docker import ROOTFS_SCRIPT_DIRS
        assert len(ROOTFS_SCRIPT_DIRS) == len(set(ROOTFS_SCRIPT_DIRS))

    def test_all_rootfs_artifacts_have_copy_in_template(self, rendered_arm64):
        """Every ROOTFS_SCRIPTS entry must have a COPY and a chmod line."""
        for artifact in ROOTFS_SCRIPTS:
            assert f"COPY {artifact} " in rendered_arm64, (
                f"{artifact} missing COPY line in Dockerfile.rootfs.j2"
            )
            assert "chmod" in rendered_arm64 and artifact in rendered_arm64, (
                f"{artifact} missing chmod line in Dockerfile.rootfs.j2"
            )

    def test_protocol_benchmark_rust_binary_is_mandatory_guest_binary(self):
        assert "capsem-bench-rs" in GUEST_BINARIES
        assert "capsem-bench" in ROOTFS_SCRIPTS


class TestCrossCompileAgent:
    """Tests for host/target-aware guest binary compilation dispatch."""

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
    @patch("capsem.builder.docker.platform.machine", return_value="aarch64")
    @patch("capsem.builder.docker.run_cmd")
    def test_native_on_linux_skips_container(
        self, mock_run, _mock_machine, mock_sys, mock_container, tmp_path,
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

    @pytest.mark.parametrize(
        ("host_machine", "rust_target"),
        [
            ("x86_64", "aarch64-unknown-linux-musl"),
            ("amd64", "aarch64-unknown-linux-musl"),
            ("aarch64", "x86_64-unknown-linux-musl"),
            ("arm64", "x86_64-unknown-linux-musl"),
        ],
    )
    @patch("capsem.builder.docker.container_compile_agent")
    @patch("capsem.builder.docker.sys")
    def test_foreign_target_on_linux_delegates_to_architecture_matched_container(
        self, mock_sys, mock_container, host_machine, rust_target, tmp_path,
    ):
        mock_sys.platform = "linux"
        mock_container.return_value = []

        with patch("capsem.builder.docker.platform.machine", return_value=host_machine):
            result = cross_compile_agent(rust_target, tmp_path, tmp_path / "out")

        assert result == []
        mock_container.assert_called_once_with(
            rust_target, tmp_path, tmp_path / "out",
        )

    @pytest.mark.parametrize(
        ("host_machine", "rust_target"),
        [
            ("aarch64", "aarch64-unknown-linux-musl"),
            ("arm64", "aarch64-unknown-linux-musl"),
            ("x86_64", "x86_64-unknown-linux-musl"),
            ("amd64", "x86_64-unknown-linux-musl"),
        ],
    )
    @patch("capsem.builder.docker.container_compile_agent")
    @patch("capsem.builder.docker.sys")
    @patch("capsem.builder.docker.run_cmd")
    def test_native_on_linux_replaces_existing_readonly_outputs(
        self, mock_run, mock_sys, mock_container,
        host_machine, rust_target, tmp_path,
    ):
        mock_sys.platform = "linux"
        mock_run.return_value = MagicMock(stdout=rust_target)

        release_dir = tmp_path / "target" / rust_target / "release"
        release_dir.mkdir(parents=True)
        output_dir = tmp_path / "out"
        output_dir.mkdir()
        for binary in GUEST_BINARIES:
            (release_dir / binary).write_bytes(f"new-{binary}".encode())
            old = output_dir / binary
            old.write_bytes(b"old")
            old.chmod(0o555)

        with patch("capsem.builder.docker.platform.machine", return_value=host_machine):
            cross_compile_agent(rust_target, tmp_path, output_dir)

        mock_container.assert_not_called()
        for binary in GUEST_BINARIES:
            dst = output_dir / binary
            assert dst.read_bytes() == f"new-{binary}".encode()
            assert dst.stat().st_mode & 0o777 == 0o555
