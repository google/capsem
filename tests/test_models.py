"""Tests for capsem.builder.models -- Pydantic models for guest image config.

TDD: these tests are written first (RED), then models.py makes them pass (GREEN).
"""

from __future__ import annotations

import pytest
from pydantic import ValidationError

from capsem.builder.models import (
    AiProviderConfig,
    ApiKeyConfig,
    ArchConfig,
    BuildConfig,
    Compression,
    FileConfig,
    GuestImageConfig,
    InstallConfig,
    McpServerConfig,
    NetworkConfig,
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


def _api_key(**kw):
    defaults = {"name": "Test Key", "env_vars": ["TEST_KEY"]}
    defaults.update(kw)
    return ApiKeyConfig(**defaults)


def _network(**kw):
    defaults = {"domains": ["*.example.com"]}
    defaults.update(kw)
    return NetworkConfig(**defaults)


def _ai_provider(**kw):
    defaults = {
        "name": "Test Provider",
        "api_key": _api_key(),
        "network": _network(),
    }
    defaults.update(kw)
    return AiProviderConfig(**defaults)


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
        assert a.kernel_branch == "6.6"
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


# ---------------------------------------------------------------------------
# ApiKeyConfig
# ---------------------------------------------------------------------------


class TestApiKeyConfig:
    def test_construction(self):
        k = _api_key(prefix="sk-", docs_url="https://example.com/keys")
        assert k.name == "Test Key"
        assert k.env_vars == ["TEST_KEY"]
        assert k.prefix == "sk-"
        assert k.docs_url == "https://example.com/keys"

    def test_defaults(self):
        k = _api_key()
        assert k.prefix == ""
        assert k.docs_url is None

    def test_empty_env_vars_rejected(self):
        with pytest.raises(ValidationError):
            ApiKeyConfig(name="Bad", env_vars=[])

    def test_multiple_env_vars(self):
        k = _api_key(env_vars=["KEY_A", "KEY_B"])
        assert len(k.env_vars) == 2


# ---------------------------------------------------------------------------
# NetworkConfig
# ---------------------------------------------------------------------------


class TestNetworkConfig:
    def test_construction(self):
        n = _network(allow_get=True, allow_post=True)
        assert n.domains == ["*.example.com"]
        assert n.allow_get is True
        assert n.allow_post is True

    def test_defaults(self):
        n = _network()
        assert n.allow_get is False
        assert n.allow_post is False

    def test_empty_domains_rejected(self):
        with pytest.raises(ValidationError):
            NetworkConfig(domains=[])

    def test_multiple_domains(self):
        n = _network(domains=["a.com", "b.com", "*.c.com"])
        assert len(n.domains) == 3


# ---------------------------------------------------------------------------
# InstallConfig
# ---------------------------------------------------------------------------


class TestInstallConfig:
    def test_construction(self):
        i = InstallConfig(manager=PackageManager.NPM, prefix="/opt/ai-clis",
                          packages=["@google/gemini-cli"])
        assert i.manager is PackageManager.NPM
        assert i.prefix == "/opt/ai-clis"
        assert i.packages == ["@google/gemini-cli"]

    def test_defaults(self):
        i = InstallConfig(manager=PackageManager.NPM, packages=["pkg"])
        assert i.prefix == ""


# ---------------------------------------------------------------------------
# FileConfig
# ---------------------------------------------------------------------------


class TestFileConfig:
    def test_construction(self):
        f = FileConfig(path="/root/.config/test.json", content='{"key":"val"}')
        assert f.path == "/root/.config/test.json"
        assert f.content == '{"key":"val"}'

    def test_empty_content(self):
        f = FileConfig(path="/root/.creds", content="")
        assert f.content == ""


# ---------------------------------------------------------------------------
# AiProviderConfig
# ---------------------------------------------------------------------------


class TestAiProviderConfig:
    def test_minimal(self):
        p = _ai_provider()
        assert p.name == "Test Provider"
        assert p.enabled is True
        assert p.install is None
        assert p.files == {}

    def test_full(self):
        p = _ai_provider(
            description="Full provider",
            enabled=False,
            install=InstallConfig(manager=PackageManager.NPM, packages=["cli"]),
            files={"settings": FileConfig(path="/root/.cfg", content="data")},
        )
        assert p.description == "Full provider"
        assert p.enabled is False
        assert p.install is not None
        assert "settings" in p.files

    def test_disabled_provider(self):
        p = _ai_provider(enabled=False)
        assert p.enabled is False
        # Validation still passes for disabled providers
        assert p.api_key.env_vars == ["TEST_KEY"]

    def test_roundtrip(self):
        p = _ai_provider(
            install=InstallConfig(manager=PackageManager.NPM, packages=["cli"]),
            files={"cfg": FileConfig(path="/a", content="b")},
        )
        data = p.model_dump()
        q = AiProviderConfig.model_validate(data)
        assert p == q


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
        assert w.allow_read is False
        assert w.allow_write is False
        assert w.custom_allow == []
        assert w.custom_block == []
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

    def test_custom_allow_block(self):
        w = WebSecurityConfig(
            custom_allow=["elie.net", "*.elie.net"],
            custom_block=["evil.com"],
        )
        assert len(w.custom_allow) == 2
        assert w.custom_block == ["evil.com"]

    def test_roundtrip(self):
        w = WebSecurityConfig(
            allow_read=True,
            custom_allow=["a.com"],
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
        assert g.ai_providers == {}
        assert g.package_sets == {}
        assert g.mcp_servers == {}
        assert g.web_security.allow_read is False
        assert g.vm_resources.cpu_count == 4
        assert g.vm_environment.shell.term == "xterm-256color"

    def test_full(self):
        g = GuestImageConfig(
            build=_build(),
            ai_providers={"google": _ai_provider(name="Google")},
            package_sets={"python": PackageSetConfig(
                name="Python", manager=PackageManager.UV,
                install_cmd="uv pip install", packages=["pytest"],
            )},
            mcp_servers={"capsem": _mcp_stdio(name="Capsem")},
            web_security=WebSecurityConfig(allow_read=True),
            vm_resources=VmResourcesConfig(cpu_count=8),
            vm_environment=VmEnvironmentConfig(
                shell=ShellConfig(term="screen"),
            ),
        )
        assert "google" in g.ai_providers
        assert "python" in g.package_sets
        assert "capsem" in g.mcp_servers
        assert g.web_security.allow_read is True
        assert g.vm_resources.cpu_count == 8
        assert g.vm_environment.shell.term == "screen"

    def test_frozen(self):
        g = GuestImageConfig(build=_build())
        with pytest.raises(ValidationError):
            g.build = _build()

    def test_json_roundtrip(self):
        g = GuestImageConfig(
            build=_build(),
            ai_providers={"test": _ai_provider()},
            mcp_servers={"mcp": _mcp_stdio()},
        )
        json_str = g.model_dump_json()
        h = GuestImageConfig.model_validate_json(json_str)
        assert g == h


# ---------------------------------------------------------------------------
# Adversarial tests
# ---------------------------------------------------------------------------


class TestAdversarial:
    def test_wildcard_domain_patterns(self):
        n = _network(domains=["*.example.com", "example.com", "*.*.deep.com"])
        assert len(n.domains) == 3

    def test_unicode_in_domain(self):
        n = _network(domains=["xn--e1afmapc.xn--p1ai"])
        assert len(n.domains) == 1

    def test_huge_package_list(self):
        packages = [f"pkg-{i}" for i in range(1000)]
        ps = PackageSetConfig(
            name="Huge", manager=PackageManager.APT,
            install_cmd="apt install", packages=packages,
        )
        assert len(ps.packages) == 1000

    def test_empty_string_content_in_file(self):
        f = FileConfig(path="/root/.empty", content="")
        assert f.content == ""

    def test_path_traversal_in_file(self):
        # Config is declarative; runtime enforces path safety
        f = FileConfig(path="../../etc/passwd", content="root:x:0:0")
        assert f.path == "../../etc/passwd"

    def test_very_long_content_in_file(self):
        content = "x" * 1_000_000
        f = FileConfig(path="/root/.big", content=content)
        assert len(f.content) == 1_000_000

    def test_special_chars_in_env_vars(self):
        k = _api_key(env_vars=["MY_KEY_123"])
        assert k.env_vars == ["MY_KEY_123"]
