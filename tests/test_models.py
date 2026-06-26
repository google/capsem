"""Tests for capsem.builder.models -- Pydantic models for guest image config.

TDD: these tests are written first (RED), then models.py makes them pass (GREEN).
"""

from __future__ import annotations

import pytest
from pydantic import ValidationError

from capsem.builder.models import (
    ArchConfig,
    BuildConfig,
    Compression,
    ErofsCompression,
    ErofsConfig,
    GuestImageConfig,
    McpServerConfig,
    PackageManager,
    PackageNetworkConfig,
    PackageSetConfig,
    ShellConfig,
    ShellFileConfig,
    TlsConfig,
    VmEnvironmentConfig,
    VmResourcesConfig,
    WebSecurityConfig,
    WebServiceConfig,
)
from capsem.builder.schema import McpTransport


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _arch(*, docker_platform="linux/arm64", rust_target="aarch64-unknown-linux-musl",
          kernel_image="arch/arm64/boot/Image", defconfig="kernel/defconfig.arm64",
          **kw):
    return ArchConfig(docker_platform=docker_platform, rust_target=rust_target,
                      kernel_image=kernel_image, defconfig=defconfig, **kw)


def _build(**kw):
    defaults = {"architectures": {"arm64": _arch()}}
    defaults.update(kw)
    return BuildConfig(**defaults)


def _mcp_stdio(**kw):
    defaults = {"name": "Test", "transport": McpTransport.STDIO, "command": "/bin/test"}
    defaults.update(kw)
    return McpServerConfig(**defaults)


# ---------------------------------------------------------------------------
# Enums
# ---------------------------------------------------------------------------


class TestCompression:
    def test_values(self):
        assert set(Compression) == {
            Compression.ZSTD, Compression.GZIP,
            Compression.LZO, Compression.XZ,
        }

    def test_string_values(self):
        assert Compression.ZSTD.value == "zstd"
        assert Compression.GZIP.value == "gzip"
        assert Compression.LZO.value == "lzo"
        assert Compression.XZ.value == "xz"

    def test_from_string(self):
        assert Compression("zstd") is Compression.ZSTD


class TestErofsCompression:
    def test_values(self):
        assert set(ErofsCompression) == {
            ErofsCompression.LZ4, ErofsCompression.LZ4HC, ErofsCompression.ZSTD,
        }

    def test_default_config_is_release_lz4hc(self):
        e = ErofsConfig()
        assert e.enabled is True
        assert e.compression is ErofsCompression.LZ4HC
        assert e.compression_level == 12
        assert e.cluster_size is None

    def test_lz4_rejects_level(self):
        with pytest.raises(ValidationError):
            ErofsConfig(compression=ErofsCompression.LZ4, compression_level=1)

    def test_lz4hc_rejects_too_high_level(self):
        with pytest.raises(ValidationError):
            ErofsConfig(compression=ErofsCompression.LZ4HC, compression_level=13)

    def test_zstd_remains_supported_option(self):
        e = ErofsConfig(compression=ErofsCompression.ZSTD, compression_level=15)
        assert e.compression is ErofsCompression.ZSTD
        assert e.compression_level == 15


class TestPackageManager:
    def test_values(self):
        assert set(PackageManager) == {
            PackageManager.APT, PackageManager.UV,
            PackageManager.PIP, PackageManager.NPM,
            PackageManager.CURL,
        }

    def test_string_values(self):
        assert PackageManager.APT.value == "apt"
        assert PackageManager.UV.value == "uv"
        assert PackageManager.PIP.value == "pip"
        assert PackageManager.NPM.value == "npm"


# ---------------------------------------------------------------------------
# ArchConfig
# ---------------------------------------------------------------------------


class TestArchConfig:
    def test_construction(self):
        a = _arch()
        assert a.docker_platform == "linux/arm64"
        assert a.rust_target == "aarch64-unknown-linux-musl"
        assert a.kernel_image == "arch/arm64/boot/Image"
        assert a.defconfig == "kernel/defconfig.arm64"

    def test_defaults(self):
        a = _arch()
        assert a.base_image == "debian:bookworm-slim"
        # "auto" -> resolver picks newest non-EOL longterm kernel from
        # kernel.org/releases.json. Pinning the digit here would cause this
        # test to fail every time a new LTS series is released; the resolver
        # itself is exercised by tests/capsem-builder/test_kernel_resolver.py.
        assert a.kernel_branch == "auto"
        assert a.node_major == 24

    def test_custom_values(self):
        a = _arch(base_image="ubuntu:24.04", kernel_branch="6.8", node_major=22)
        assert a.base_image == "ubuntu:24.04"
        assert a.kernel_branch == "6.8"
        assert a.node_major == 22

    def test_frozen(self):
        a = _arch()
        with pytest.raises(ValidationError):
            a.base_image = "other"

    def test_roundtrip(self):
        a = _arch()
        data = a.model_dump()
        b = ArchConfig.model_validate(data)
        assert a == b


# ---------------------------------------------------------------------------
# BuildConfig
# ---------------------------------------------------------------------------


class TestBuildConfig:
    def test_defaults(self):
        b = _build()
        assert b.compression is Compression.ZSTD
        assert b.compression_level == 15
        assert b.erofs.compression is ErofsCompression.LZ4HC
        assert b.erofs.compression_level == 12

    def test_compression_level_min(self):
        b = _build(compression_level=1)
        assert b.compression_level == 1

    def test_compression_level_max(self):
        b = _build(compression_level=22)
        assert b.compression_level == 22

    def test_compression_level_too_low(self):
        with pytest.raises(ValidationError):
            _build(compression_level=0)

    def test_compression_level_too_high(self):
        with pytest.raises(ValidationError):
            _build(compression_level=23)

    def test_empty_architectures_rejected(self):
        with pytest.raises(ValidationError):
            BuildConfig(architectures={})

    def test_single_arch(self):
        b = _build()
        assert "arm64" in b.architectures
        assert len(b.architectures) == 1

    def test_multi_arch(self):
        x86 = ArchConfig(
            docker_platform="linux/amd64",
            rust_target="x86_64-unknown-linux-musl",
            kernel_image="arch/x86_64/boot/bzImage",
            defconfig="kernel/defconfig.x86_64",
        )
        b = _build(architectures={"arm64": _arch(), "x86_64": x86})
        assert len(b.architectures) == 2
        assert "x86_64" in b.architectures

    def test_roundtrip(self):
        b = _build()
        data = b.model_dump()
        c = BuildConfig.model_validate(data)
        assert b == c

    def test_version_commands_default(self):
        b = _build()
        assert b.version_commands == {}

    def test_version_commands(self):
        b = _build(version_commands={"node": "node --version", "npm": "npm --version"})
        assert b.version_commands["node"] == "node --version"
        assert len(b.version_commands) == 2

    def test_version_commands_roundtrip(self):
        b = _build(version_commands={"uv": "uv --version"})
        data = b.model_dump()
        c = BuildConfig.model_validate(data)
        assert b == c


# ---------------------------------------------------------------------------
# PackageSetConfig
# ---------------------------------------------------------------------------


class TestPackageSetConfig:
    def test_minimal(self):
        ps = PackageSetConfig(
            name="Python", manager=PackageManager.UV,
            install_cmd="uv pip install --system", packages=["pytest"],
        )
        assert ps.name == "Python"
        assert ps.manager is PackageManager.UV
        assert ps.network is None

    def test_with_network(self):
        net = PackageNetworkConfig(name="PyPI", domains=["pypi.org"])
        ps = PackageSetConfig(
            name="Python", manager=PackageManager.UV,
            install_cmd="uv pip install", packages=["pytest"], network=net,
        )
        assert ps.network is not None
        assert ps.network.name == "PyPI"

    def test_empty_packages_rejected(self):
        with pytest.raises(ValidationError):
            PackageSetConfig(
                name="Empty", manager=PackageManager.APT,
                install_cmd="apt install", packages=[],
            )

    def test_empty_install_cmd_rejected(self):
        with pytest.raises(ValidationError):
            PackageSetConfig(
                name="Bad", manager=PackageManager.APT,
                install_cmd="", packages=["pkg"],
            )

    def test_version_commands_default(self):
        ps = PackageSetConfig(
            name="Test", manager=PackageManager.APT,
            install_cmd="apt install", packages=["git"],
        )
        assert ps.version_commands == {}

    def test_version_commands_valid(self):
        ps = PackageSetConfig(
            name="Test", manager=PackageManager.APT,
            install_cmd="apt install", packages=["git", "curl"],
            version_commands={"git": "git --version"},
        )
        assert ps.version_commands["git"] == "git --version"

    def test_version_commands_unknown_key_rejected(self):
        with pytest.raises(ValidationError, match="version_commands keys not in packages"):
            PackageSetConfig(
                name="Bad", manager=PackageManager.APT,
                install_cmd="apt install", packages=["git"],
                version_commands={"nonexistent": "echo 1"},
            )

    def test_roundtrip(self):
        ps = PackageSetConfig(
            name="Node", manager=PackageManager.NPM,
            install_cmd="npm install -g", packages=["typescript"],
        )
        data = ps.model_dump()
        q = PackageSetConfig.model_validate(data)
        assert ps == q


# ---------------------------------------------------------------------------
# McpServerConfig
# ---------------------------------------------------------------------------


class TestMcpServerConfig:
    def test_stdio_transport(self):
        m = _mcp_stdio()
        assert m.transport is McpTransport.STDIO
        assert m.command == "/bin/test"

    def test_sse_transport(self):
        m = McpServerConfig(
            name="SSE", transport=McpTransport.SSE,
            url="http://localhost:8080",
        )
        assert m.transport is McpTransport.SSE
        assert m.url == "http://localhost:8080"

    def test_stdio_without_command_rejected(self):
        with pytest.raises(ValidationError):
            McpServerConfig(name="Bad", transport=McpTransport.STDIO)

    def test_sse_without_url_rejected(self):
        with pytest.raises(ValidationError):
            McpServerConfig(name="Bad", transport=McpTransport.SSE)

    def test_builtin(self):
        m = _mcp_stdio(builtin=True)
        assert m.builtin is True

    def test_with_args_env_headers(self):
        m = _mcp_stdio(
            args=["--verbose"],
            env={"DEBUG": "1"},
            headers={"Authorization": "Bearer tok"},
        )
        assert m.args == ["--verbose"]
        assert m.env == {"DEBUG": "1"}
        assert m.headers == {"Authorization": "Bearer tok"}

    def test_mcptransport_reused_from_schema(self):
        """McpTransport is imported from schema.py, not duplicated."""
        from capsem.builder.schema import McpTransport as SchemaMcpTransport
        assert McpTransport is SchemaMcpTransport

    def test_defaults(self):
        m = _mcp_stdio()
        assert m.description == ""
        assert m.args == []
        assert m.env == {}
        assert m.headers == {}
        assert m.builtin is False
        assert m.enabled is True

    def test_roundtrip(self):
        m = _mcp_stdio(args=["--flag"], env={"K": "V"})
        data = m.model_dump()
        n = McpServerConfig.model_validate(data)
        assert m == n


# ---------------------------------------------------------------------------
# WebServiceConfig
# ---------------------------------------------------------------------------


class TestWebServiceConfig:
    def test_defaults(self):
        w = WebServiceConfig(name="Test", domains=["example.com"])
        assert w.enabled is True
        assert w.allow_get is False
        assert w.allow_post is False

    def test_full(self):
        w = WebServiceConfig(
            name="Google", enabled=True,
            domains=["google.com", "www.google.com"],
            allow_get=True, allow_post=False,
        )
        assert len(w.domains) == 2
        assert w.allow_get is True


# ---------------------------------------------------------------------------
# WebSecurityConfig
# ---------------------------------------------------------------------------


class TestWebSecurityConfig:
    def test_defaults(self):
        w = WebSecurityConfig()
        assert w.http_upstream_ports == [80, 3128, 3713, 8080, 11434]
        assert w.search == {}
        assert w.registry == {}
        assert w.repository == {}

    def test_with_services(self):
        google = WebServiceConfig(
            name="Google", domains=["google.com"], allow_get=True,
        )
        pypi = WebServiceConfig(
            name="PyPI", domains=["pypi.org"], allow_get=True,
        )
        w = WebSecurityConfig(
            search={"google": google},
            registry={"pypi": pypi},
        )
        assert "google" in w.search
        assert "pypi" in w.registry

    def test_retired_decision_fields_forbidden(self):
        with pytest.raises(ValidationError):
            WebSecurityConfig(
                allow_read=True,
                allow_write=True,
                custom_allow=["elie.net", "*.elie.net"],
                custom_block=["evil.com"],
            )

    def test_roundtrip(self):
        w = WebSecurityConfig(
            http_upstream_ports=[80],
            search={"g": WebServiceConfig(name="G", domains=["g.com"])},
        )
        data = w.model_dump()
        x = WebSecurityConfig.model_validate(data)
        assert w == x


# ---------------------------------------------------------------------------
# VmResourcesConfig
# ---------------------------------------------------------------------------


class TestVmResourcesConfig:
    def test_defaults(self):
        r = VmResourcesConfig()
        assert r.cpu_count == 4
        assert r.ram_gb == 4
        assert r.scratch_disk_size_gb == 16
        assert r.log_bodies is False
        assert r.max_body_capture == 4096
        assert r.retention_days == 30
        assert r.max_sessions == 100
        assert r.max_disk_gb == 100
        assert r.terminated_retention_days == 365

    def test_min_bounds(self):
        r = VmResourcesConfig(
            cpu_count=1, ram_gb=1, scratch_disk_size_gb=1,
            max_body_capture=0, retention_days=1, max_sessions=1,
            max_disk_gb=1, terminated_retention_days=30,
        )
        assert r.cpu_count == 1

    def test_max_bounds(self):
        r = VmResourcesConfig(
            cpu_count=8, ram_gb=16, scratch_disk_size_gb=128,
            max_body_capture=1048576, retention_days=365, max_sessions=10000,
            max_disk_gb=1000, terminated_retention_days=3650,
        )
        assert r.cpu_count == 8

    def test_cpu_count_too_low(self):
        with pytest.raises(ValidationError):
            VmResourcesConfig(cpu_count=0)

    def test_cpu_count_too_high(self):
        with pytest.raises(ValidationError):
            VmResourcesConfig(cpu_count=9)

    def test_ram_too_high(self):
        with pytest.raises(ValidationError):
            VmResourcesConfig(ram_gb=17)

    def test_terminated_retention_too_low(self):
        with pytest.raises(ValidationError):
            VmResourcesConfig(terminated_retention_days=29)

    def test_roundtrip(self):
        r = VmResourcesConfig(cpu_count=2, ram_gb=8)
        data = r.model_dump()
        s = VmResourcesConfig.model_validate(data)
        assert r == s


# ---------------------------------------------------------------------------
# VmEnvironmentConfig
# ---------------------------------------------------------------------------


class TestVmEnvironmentConfig:
    def test_defaults(self):
        e = VmEnvironmentConfig()
        assert e.shell.term == "xterm-256color"
        assert e.shell.home == "/root"
        assert e.shell.lang == "C"
        assert e.tls.ca_bundle == "/etc/ssl/certs/ca-certificates.crt"

    def test_shell_path_default(self):
        e = VmEnvironmentConfig()
        assert "/usr/bin" in e.shell.path
        assert "/opt/ai-clis/bin" in e.shell.path

    def test_with_shell_files(self):
        bashrc = ShellFileConfig(path="/root/.bashrc", content="PS1='$ '")
        tmux = ShellFileConfig(path="/root/.tmux.conf", content="set -g mouse on")
        shell = ShellConfig(bashrc=bashrc, tmux_conf=tmux)
        e = VmEnvironmentConfig(shell=shell)
        assert e.shell.bashrc is not None
        assert e.shell.tmux_conf is not None
        assert e.shell.bashrc.content == "PS1='$ '"

    def test_without_shell_files(self):
        e = VmEnvironmentConfig()
        assert e.shell.bashrc is None
        assert e.shell.tmux_conf is None

    def test_custom_tls(self):
        tls = TlsConfig(ca_bundle="/custom/ca.crt")
        e = VmEnvironmentConfig(tls=tls)
        assert e.tls.ca_bundle == "/custom/ca.crt"

    def test_roundtrip(self):
        e = VmEnvironmentConfig(
            shell=ShellConfig(
                term="screen",
                bashrc=ShellFileConfig(path="/root/.bashrc", content="# hi"),
            ),
        )
        data = e.model_dump()
        f = VmEnvironmentConfig.model_validate(data)
        assert e == f


# ---------------------------------------------------------------------------
# GuestImageConfig
# ---------------------------------------------------------------------------


class TestGuestImageConfig:
    def test_minimal(self):
        g = GuestImageConfig(build=_build())
        assert g.build.compression is Compression.ZSTD
        assert g.package_sets == {}
        assert g.mcp_servers == {}
        assert g.web_security.http_upstream_ports == [80, 3128, 3713, 8080, 11434]
        assert g.vm_resources.cpu_count == 4
        assert g.vm_environment.shell.term == "xterm-256color"

    def test_full(self):
        g = GuestImageConfig(
            build=_build(),
            package_sets={"python": PackageSetConfig(
                name="Python", manager=PackageManager.UV,
                install_cmd="uv pip install", packages=["pytest"],
            )},
            mcp_servers={"capsem": _mcp_stdio(name="Capsem")},
            web_security=WebSecurityConfig(http_upstream_ports=[80]),
            vm_resources=VmResourcesConfig(cpu_count=8),
            vm_environment=VmEnvironmentConfig(
                shell=ShellConfig(term="screen"),
            ),
        )
        assert "python" in g.package_sets
        assert "capsem" in g.mcp_servers
        assert g.web_security.http_upstream_ports == [80]
        assert g.vm_resources.cpu_count == 8
        assert g.vm_environment.shell.term == "screen"

    def test_frozen(self):
        g = GuestImageConfig(build=_build())
        with pytest.raises(ValidationError):
            g.build = _build()

    def test_json_roundtrip(self):
        g = GuestImageConfig(
            build=_build(),
            mcp_servers={"mcp": _mcp_stdio()},
        )
        json_str = g.model_dump_json()
        h = GuestImageConfig.model_validate_json(json_str)
        assert g == h


# ---------------------------------------------------------------------------
# Adversarial tests
# ---------------------------------------------------------------------------


class TestAdversarial:
    def test_huge_package_list(self):
        packages = [f"pkg-{i}" for i in range(1000)]
        ps = PackageSetConfig(
            name="Huge", manager=PackageManager.APT,
            install_cmd="apt install", packages=packages,
        )
        assert len(ps.packages) == 1000
