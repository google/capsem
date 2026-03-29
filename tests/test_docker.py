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
    def test_podman_found(self, mock_check):
        from capsem.builder.doctor import CheckResult
        mock_check.return_value = CheckResult(
            name="container-runtime", passed=True, detail="podman version 5.3.1"
        )
        assert detect_runtime() == "podman"

    @patch("capsem.builder.docker.check_container_runtime")
    def test_docker_found(self, mock_check):
        from capsem.builder.doctor import CheckResult
        mock_check.return_value = CheckResult(
            name="container-runtime", passed=True, detail="Docker version 27.1.1"
        )
        assert detect_runtime() == "docker"

    @patch("capsem.builder.docker.check_container_runtime")
    def test_neither_raises(self, mock_check):
        from capsem.builder.doctor import CheckResult
        mock_check.return_value = CheckResult(
            name="container-runtime", passed=False,
            detail="not found", fix="install podman",
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
# Build execution: docker_build
# ---------------------------------------------------------------------------


class TestDockerBuild:
    @patch("capsem.builder.docker.run_cmd")
    def test_regular_build(self, mock_run):
        docker_build(
            runtime="podman", tag="test-image", dockerfile_path="/tmp/Dockerfile",
            context_dir="/tmp/ctx", platform="linux/arm64",
        )
        cmd = mock_run.call_args[0][0]
        assert cmd[0] == "podman"
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
    def test_podman_no_buildx(self, mock_run):
        docker_build(
            runtime="podman", tag="test-image", dockerfile_path="/tmp/Dockerfile",
            context_dir="/tmp/ctx", platform="linux/arm64", ci_cache=True,
        )
        cmd = mock_run.call_args[0][0]
        # Podman ignores ci_cache, uses plain build
        assert cmd[0] == "podman"
        assert "buildx" not in cmd

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
        extract_kernel_assets("podman", "kernel-img", "linux/arm64", out)
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


class TestCreateSquashfs:
    @patch("capsem.builder.docker.run_cmd")
    def test_zstd_compression(self, mock_run):
        create_squashfs(
            "podman", Path("/tmp/rootfs.tar"), Path("/tmp/rootfs.squashfs"),
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
