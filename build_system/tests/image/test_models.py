"""Tests for capsem_builder.image.models -- Pydantic models for guest image config.

TDD: these tests are written first (RED), then models.py makes them pass (GREEN).
"""

from __future__ import annotations

from typing import cast

import pytest
from capsem_builder.image.models import (
    ArchConfig,
    AssetDependencyArchitectureConfig,
    AssetDependencyConfig,
    AssetToolBinaryConfig,
    AssetToolsArchitectureConfig,
    AssetToolsConfig,
    BuildConfig,
    ErofsCompression,
    ErofsConfig,
    GuestImageConfig,
    GuestRustBuilderConfig,
    KernelConfig,
    McpServerConfig,
    NodeDownloadConfig,
    PackageManager,
    PackageNetworkConfig,
    PackageSetConfig,
    RootfsConfig,
    ShellConfig,
    ShellFileConfig,
    TlsConfig,
    VersionedDownloadConfig,
    VmEnvironmentConfig,
    VmResourcesConfig,
    WebSecurityConfig,
    WebServiceConfig,
)
from capsem_builder.image.schema import McpTransport
from capsem_builder.policy.dockerpolicy import BuildNetwork, ContainerNetwork
from pydantic import ValidationError

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _arch(
    *,
    docker_platform="linux/arm64",
    rust_target="aarch64-unknown-linux-musl",
    kernel_image="arch/arm64/boot/Image",
    defconfig="kernel/defconfig.arm64",
    **kw,
):
    kw.setdefault("base_image", "docker.io/library/debian@sha256:" + "a" * 64)
    kw.setdefault(
        "rust_builder_base_image",
        "docker.io/library/rust@sha256:" + "c" * 64,
    )
    return ArchConfig(
        docker_platform=docker_platform,
        rust_target=rust_target,
        kernel_image=kernel_image,
        defconfig=defconfig,
        **kw,
    )


def _build(**kw):
    binary = AssetToolBinaryConfig(url="https://example.test/tool", sha256="d" * 64)
    architectures = cast(dict[str, ArchConfig], kw.get("architectures", {"arm64": _arch()}))
    asset_tools = cast(AssetToolsConfig | None, kw.get("asset_tools"))
    if asset_tools is None:
        asset_tools = AssetToolsConfig(
            dockerfile="docker/Dockerfile.asset-tools",
            tag_template="capsem-asset-tools-{arch}:{digest}",
            debian_snapshot_base="http://snapshot.example/debian",
            debian_security_snapshot_base="http://snapshot.example/debian-security",
            debian_snapshot_id="20260810T000000Z",
            materialize_network=BuildNetwork.DEFAULT,
            runtime_network=ContainerNetwork.NONE,
            architectures={
                name: AssetToolsArchitectureConfig(
                    cdxgen=binary,
                    cdx_validate=binary,
                )
                for name in architectures
            },
        )
    defaults: dict[str, object] = {
        "materialize_network": BuildNetwork.DEFAULT,
        "asset_dependencies": AssetDependencyConfig(
            tag_template="capsem-{template}-dependencies-{arch}:{digest}",
            rootfs_template="Dockerfile.rootfs-dependencies.j2",
            kernel_template="Dockerfile.kernel-dependencies.j2",
            source_build_network=BuildNetwork.NONE,
            architectures={name: _dependency_arch(name) for name in architectures},
        ),
        "kernel": KernelConfig(version="9.9.9", sha256="a" * 64),
        "rootfs": RootfsConfig(
            max_uncompressed_bytes=2_500_000_000,
            max_erofs_bytes=900_000_000,
            forbidden_path_prefixes=("usr/lib/ollama/cuda_",),
        ),
        "guest_rust_builder": GuestRustBuilderConfig(
            dockerfile="docker/Dockerfile.guest-rust-builder",
            tag_template="capsem-guest-rust-{arch}:{digest}",
            identity_inputs=("Cargo.lock", "rust-toolchain.toml"),
            cross_packages=("clang21=21.1.2-r2",),
            runtime_network=ContainerNetwork.NONE,
        ),
        "asset_tools": asset_tools,
        "architectures": architectures,
    }
    defaults.update(kw)
    return BuildConfig.model_validate(defaults)


def _dependency_arch(name: str) -> AssetDependencyArchitectureConfig:
    suffix = "arm64" if name == "arm64" else "x64"
    download = VersionedDownloadConfig(
        version="1.2.3",
        url=f"https://example.test/tool-1.2.3-linux-{suffix}",
        sha256="f" * 64,
    )
    return AssetDependencyArchitectureConfig(
        node=NodeDownloadConfig(
            version="24.19.0",
            url=f"https://example.test/node-v24.19.0-linux-{suffix}.tar.xz",
            sha256="e" * 64,
            npm_version="11.17.0",
        ),
        uv=download,
        claude=download,
        ollama=download,
    )


def _mcp_stdio(**kw):
    defaults = {"name": "Test", "transport": McpTransport.STDIO, "command": "/bin/test"}
    defaults.update(kw)
    return McpServerConfig(**defaults)


def test_asset_dependency_node_major_must_match_architecture() -> None:
    dependency = _dependency_arch("arm64")
    bad_node = dependency.node.model_copy(update={"version": "23.11.0"})
    with pytest.raises(ValidationError, match="Node major"):
        _build(
            asset_dependencies=AssetDependencyConfig(
                tag_template="capsem-{template}-dependencies-{arch}:{digest}",
                rootfs_template="Dockerfile.rootfs-dependencies.j2",
                kernel_template="Dockerfile.kernel-dependencies.j2",
                source_build_network=BuildNetwork.NONE,
                architectures={"arm64": dependency.model_copy(update={"node": bad_node})},
            )
        )


# ---------------------------------------------------------------------------
# Enums
# ---------------------------------------------------------------------------


class TestErofsCompression:
    def test_values(self):
        assert set(ErofsCompression) == {
            ErofsCompression.LZ4,
            ErofsCompression.LZ4HC,
        }

    def test_zstd_is_not_an_erofs_release_format(self):
        with pytest.raises(ValueError):
            ErofsCompression("zstd")

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

    @pytest.mark.parametrize("cluster_size", (0, 4095, 4097, 1048577))
    def test_cluster_size_is_a_bounded_power_of_two(self, cluster_size):
        with pytest.raises(ValidationError, match="cluster_size"):
            ErofsConfig(cluster_size=cluster_size)


class TestRootfsConfig:
    def test_release_limits_and_forbidden_payloads_are_typed(self):
        config = RootfsConfig(
            max_uncompressed_bytes=2_500_000_000,
            max_erofs_bytes=900_000_000,
            forbidden_path_prefixes=("usr/lib/ollama/cuda_",),
        )

        assert config.max_uncompressed_bytes == 2_500_000_000
        assert config.max_erofs_bytes == 900_000_000

    @pytest.mark.parametrize(
        "prefix",
        (
            "/usr/lib/ollama",
            "../usr/lib/ollama",
            "usr/lib/../ollama",
            " usr/lib/ollama",
            "",
        ),
    )
    def test_forbidden_prefixes_must_be_safe_relative_paths(self, prefix: str):
        with pytest.raises(ValidationError):
            RootfsConfig(
                max_uncompressed_bytes=2_500_000_000,
                max_erofs_bytes=900_000_000,
                forbidden_path_prefixes=(prefix,),
            )

    @pytest.mark.parametrize(
        "overrides",
        (
            {"max_uncompressed_bytes": 900_000_000},
            {"max_erofs_bytes": 2_500_000_000},
            {"forbidden_path_prefixes": ()},
            {
                "forbidden_path_prefixes": (
                    "usr/lib/ollama/cuda",
                    "usr/lib/ollama/cuda",
                )
            },
        ),
    )
    def test_release_limits_cannot_be_ambiguous_or_inverted(self, overrides: dict):
        values = {
            "max_uncompressed_bytes": 2_500_000_000,
            "max_erofs_bytes": 900_000_000,
            "forbidden_path_prefixes": ("usr/lib/ollama/cuda",),
            **overrides,
        }
        with pytest.raises(ValidationError):
            RootfsConfig.model_validate(values)


class TestPackageManager:
    def test_values(self):
        assert set(PackageManager) == {
            PackageManager.APT,
            PackageManager.UV,
            PackageManager.PIP,
            PackageManager.NPM,
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
        assert a.base_image == "docker.io/library/debian@sha256:" + "a" * 64
        assert a.rust_builder_base_image == "docker.io/library/rust@sha256:" + "c" * 64
        assert a.node_major == 24

    def test_custom_values(self):
        image = "registry.example/guest@sha256:" + "b" * 64
        a = _arch(base_image=image, node_major=22)
        assert a.base_image == image
        assert a.node_major == 22

    @pytest.mark.parametrize(
        "base_image",
        [
            "debian:bookworm-slim",
            "debian@sha256:short",
            "debian@sha256:" + "A" * 64,
            "sha256:" + "a" * 64,
        ],
    )
    def test_mutable_or_malformed_base_image_is_rejected(self, base_image: str) -> None:
        with pytest.raises(ValidationError):
            _arch(base_image=base_image)

    def test_base_image_is_required(self) -> None:
        with pytest.raises(ValidationError, match="base_image"):
            ArchConfig.model_validate(
                {
                    "docker_platform": "linux/arm64",
                    "rust_target": "aarch64-unknown-linux-musl",
                    "kernel_image": "arch/arm64/boot/Image",
                    "defconfig": "kernel/defconfig.arm64",
                }
            )

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
        assert b.erofs.compression is ErofsCompression.LZ4HC
        assert b.erofs.compression_level == 12

    @pytest.mark.parametrize(
        "retired",
        [
            {"compression": "zstd"},
            {"compression_level": 15},
        ],
    )
    def test_retired_archive_compression_fields_are_rejected(self, retired):
        with pytest.raises(ValidationError, match=next(iter(retired))):
            _build(**retired)

    def test_kernel_source_is_exact_and_digest_verified(self):
        kernel = _build().kernel
        assert kernel.version == "9.9.9"
        assert kernel.sha256 == "a" * 64

    @pytest.mark.parametrize(
        ("version", "sha256"),
        [
            ("9.9", "a" * 64),
            ("latest", "a" * 64),
            ("9.9.9", "A" * 64),
            ("9.9.9", "short"),
        ],
    )
    def test_kernel_source_rejects_mutable_or_unverified_inputs(
        self, version: str, sha256: str
    ) -> None:
        with pytest.raises(ValidationError):
            KernelConfig(version=version, sha256=sha256)

    def test_legacy_per_arch_kernel_branch_is_rejected(self) -> None:
        with pytest.raises(ValidationError, match="kernel_branch"):
            ArchConfig.model_validate(
                {
                    "base_image": "registry.example/debian@sha256:" + "a" * 64,
                    "docker_platform": "linux/arm64",
                    "rust_target": "aarch64-unknown-linux-musl",
                    "kernel_image": "arch/arm64/boot/Image",
                    "defconfig": "kernel/defconfig.arm64",
                    "kernel_branch": "9.9",
                }
            )

    def test_empty_architectures_rejected(self):
        with pytest.raises(ValidationError):
            _build(architectures={})

    def test_single_arch(self):
        b = _build()
        assert "arm64" in b.architectures
        assert len(b.architectures) == 1

    def test_multi_arch(self):
        x86 = ArchConfig(
            base_image="registry.example/debian@sha256:" + "b" * 64,
            rust_builder_base_image="registry.example/rust@sha256:" + "c" * 64,
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
            name="Python",
            manager=PackageManager.UV,
            install_cmd="uv pip install --system",
            packages=["pytest"],
        )
        assert ps.name == "Python"
        assert ps.manager is PackageManager.UV
        assert ps.network is None

    def test_with_network(self):
        net = PackageNetworkConfig(name="PyPI", domains=["pypi.org"])
        ps = PackageSetConfig(
            name="Python",
            manager=PackageManager.UV,
            install_cmd="uv pip install",
            packages=["pytest"],
            network=net,
        )
        assert ps.network is not None
        assert ps.network.name == "PyPI"

    def test_empty_packages_rejected(self):
        with pytest.raises(ValidationError):
            PackageSetConfig(
                name="Empty",
                manager=PackageManager.APT,
                install_cmd="apt install",
                packages=[],
            )

    def test_empty_install_cmd_rejected(self):
        with pytest.raises(ValidationError):
            PackageSetConfig(
                name="Bad",
                manager=PackageManager.APT,
                install_cmd="",
                packages=["pkg"],
            )

    def test_version_commands_default(self):
        ps = PackageSetConfig(
            name="Test",
            manager=PackageManager.APT,
            install_cmd="apt install",
            packages=["git"],
        )
        assert ps.version_commands == {}

    def test_version_commands_valid(self):
        ps = PackageSetConfig(
            name="Test",
            manager=PackageManager.APT,
            install_cmd="apt install",
            packages=["git", "curl"],
            version_commands={"git": "git --version"},
        )
        assert ps.version_commands["git"] == "git --version"

    def test_version_commands_unknown_key_rejected(self):
        with pytest.raises(ValidationError, match="version_commands keys not in packages"):
            PackageSetConfig(
                name="Bad",
                manager=PackageManager.APT,
                install_cmd="apt install",
                packages=["git"],
                version_commands={"nonexistent": "echo 1"},
            )

    def test_roundtrip(self):
        ps = PackageSetConfig(
            name="Node",
            manager=PackageManager.NPM,
            install_cmd="npm install -g",
            packages=["typescript"],
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
            name="SSE",
            transport=McpTransport.SSE,
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
        from capsem_builder.image.schema import McpTransport as SchemaMcpTransport

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
            name="Google",
            enabled=True,
            domains=["google.com", "www.google.com"],
            allow_get=True,
            allow_post=False,
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
            name="Google",
            domains=["google.com"],
            allow_get=True,
        )
        pypi = WebServiceConfig(
            name="PyPI",
            domains=["pypi.org"],
            allow_get=True,
        )
        w = WebSecurityConfig(
            search={"google": google},
            registry={"pypi": pypi},
        )
        assert "google" in w.search
        assert "pypi" in w.registry

    def test_retired_decision_fields_forbidden(self):
        with pytest.raises(ValidationError):
            # ty: ignore[unknown-argument] -- passing the retired fields is the
            # point: the model must reject them rather than accept and ignore.
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
            cpu_count=1,
            ram_gb=1,
            scratch_disk_size_gb=1,
            max_body_capture=0,
            retention_days=1,
            max_sessions=1,
            max_disk_gb=1,
            terminated_retention_days=30,
        )
        assert r.cpu_count == 1

    def test_max_bounds(self):
        r = VmResourcesConfig(
            cpu_count=8,
            ram_gb=16,
            scratch_disk_size_gb=128,
            max_body_capture=1048576,
            retention_days=365,
            max_sessions=10000,
            max_disk_gb=1000,
            terminated_retention_days=3650,
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
        assert g.build.erofs.compression is ErofsCompression.LZ4HC
        assert g.package_sets == {}
        assert g.mcp_servers == {}
        assert g.web_security.http_upstream_ports == [80, 3128, 3713, 8080, 11434]
        assert g.vm_resources.cpu_count == 4
        assert g.vm_environment.shell.term == "xterm-256color"

    def test_full(self):
        g = GuestImageConfig(
            build=_build(),
            package_sets={
                "python": PackageSetConfig(
                    name="Python",
                    manager=PackageManager.UV,
                    install_cmd="uv pip install",
                    packages=["pytest"],
                )
            },
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
            name="Huge",
            manager=PackageManager.APT,
            install_cmd="apt install",
            packages=packages,
        )
        assert len(ps.packages) == 1000
