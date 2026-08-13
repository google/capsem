"""Capsem build configuration models -- Pydantic backend image spec models.

These models define the structure of the admin-materialized backend image
workspace. Distinct from schema.py which defines the settings interchange
format.
"""

from __future__ import annotations

from enum import Enum
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field, model_validator

from capsem.builder.schema import McpTransport
from capsem.dockerpolicy import BuildNetwork, ContainerNetwork

# ---------------------------------------------------------------------------
# Enums
# ---------------------------------------------------------------------------


class ErofsCompression(str, Enum):
    """Compression algorithm for EROFS rootfs assets."""

    LZ4 = "lz4"
    LZ4HC = "lz4hc"


class PackageManager(str, Enum):
    """Package manager for installing packages."""

    APT = "apt"
    UV = "uv"
    PIP = "pip"
    NPM = "npm"
    CURL = "curl"


# ---------------------------------------------------------------------------
# Build configuration
# ---------------------------------------------------------------------------


class ArchConfig(BaseModel):
    """Per-architecture build settings."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    base_image: str = Field(pattern=r"^[^@\s]+@sha256:[0-9a-f]{64}$")
    rust_builder_base_image: str = Field(pattern=r"^[^@\s]+@sha256:[0-9a-f]{64}$")
    docker_platform: str
    rust_target: str
    kernel_image: str
    defconfig: str
    node_major: int = 24


class KernelConfig(BaseModel):
    """Immutable guest-kernel source selected by the checked-in build contract."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    version: str = Field(pattern=r"^\d+\.\d+\.\d+$")
    sha256: str = Field(pattern=r"^[0-9a-f]{64}$")


class GuestRustBuilderConfig(BaseModel):
    """Input-keyed image cache that owns guest Rust build dependencies."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    dockerfile: str
    tag_template: str
    identity_inputs: tuple[str, ...]
    cross_packages: tuple[str, ...]
    runtime_network: Literal[ContainerNetwork.NONE]

    @model_validator(mode="after")
    def _identity_is_complete(self):
        if not self.identity_inputs:
            raise ValueError("identity_inputs must have at least one entry")
        if not self.cross_packages:
            raise ValueError("cross_packages must have at least one exact package")
        if any(
            "=" not in package or any(ch.isspace() for ch in package)
            for package in self.cross_packages
        ):
            raise ValueError("cross_packages must use exact name=version package specs")
        if "{arch}" not in self.tag_template or "{digest}" not in self.tag_template:
            raise ValueError("tag_template must contain {arch} and {digest}")
        return self


class AssetToolBinaryConfig(BaseModel):
    """One upstream standalone binary admitted to the asset helper."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    url: str = Field(pattern=r"^https://")
    sha256: str = Field(pattern=r"^[0-9a-f]{64}$")


class AssetToolsArchitectureConfig(BaseModel):
    """Host-architecture downloads for the asset post-processing helper."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    cdxgen: AssetToolBinaryConfig
    cdx_validate: AssetToolBinaryConfig


class AssetToolsConfig(BaseModel):
    """Input-keyed EROFS/OBOM helper materialized before asset lanes."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    dockerfile: str
    tag_template: str
    debian_snapshot_base: str
    debian_security_snapshot_base: str
    debian_snapshot_id: str = Field(pattern=r"^\d{8}T\d{6}Z$")
    materialize_network: Literal[BuildNetwork.DEFAULT]
    runtime_network: Literal[ContainerNetwork.NONE]
    architectures: dict[str, AssetToolsArchitectureConfig]

    @model_validator(mode="after")
    def _identity_is_complete(self):
        if "{arch}" not in self.tag_template or "{digest}" not in self.tag_template:
            raise ValueError("tag_template must contain {arch} and {digest}")
        if not self.architectures:
            raise ValueError("asset tool architectures must not be empty")
        return self


class VersionedDownloadConfig(BaseModel):
    """One exact third-party download admitted to a dependency helper."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    version: str = Field(min_length=1, pattern=r"^[A-Za-z0-9][A-Za-z0-9.+-]*$")
    url: str = Field(pattern=r"^https://")
    sha256: str = Field(pattern=r"^[0-9a-f]{64}$")

    @model_validator(mode="after")
    def _url_names_version(self):
        if self.version not in self.url:
            raise ValueError("versioned download URL must contain its exact version")
        return self


class NodeDownloadConfig(VersionedDownloadConfig):
    """Exact Node archive plus the npm version bundled inside it."""

    version: str = Field(pattern=r"^\d+\.\d+\.\d+$")
    npm_version: str = Field(pattern=r"^\d+\.\d+\.\d+$")


class AssetDependencyArchitectureConfig(BaseModel):
    """Exact network inputs for one guest architecture's rootfs helper."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    node: NodeDownloadConfig
    uv: VersionedDownloadConfig
    claude: VersionedDownloadConfig
    ollama: VersionedDownloadConfig


class AssetDependencyConfig(BaseModel):
    """Network-open helpers consumed by sealed kernel/rootfs source builds."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    tag_template: str
    rootfs_template: str
    kernel_template: str
    source_build_network: Literal[BuildNetwork.NONE]
    architectures: dict[str, AssetDependencyArchitectureConfig]

    @model_validator(mode="after")
    def _templates_are_complete(self):
        for field in ("{template}", "{arch}", "{digest}"):
            if field not in self.tag_template:
                raise ValueError(f"tag_template must contain {field}")
        if self.rootfs_template == self.kernel_template:
            raise ValueError("rootfs and kernel dependency templates must differ")
        if not self.architectures:
            raise ValueError("asset dependency architectures must not be empty")
        return self


class ErofsConfig(BaseModel):
    """EROFS rootfs asset settings.

    EROFS is the 1.3 rootfs asset path and defaults to lz4hc level 12 based on
    macOS/Linux benchmarks.
    """

    model_config = ConfigDict(frozen=True)

    enabled: bool = True
    compression: ErofsCompression = ErofsCompression.LZ4HC
    compression_level: int | None = 12
    cluster_size: int | None = None

    @model_validator(mode="after")
    def _compression_level_valid(self):
        if self.compression is ErofsCompression.LZ4:
            if self.compression_level is not None:
                raise ValueError("lz4 EROFS compression does not accept a level")
        elif self.compression is ErofsCompression.LZ4HC:
            if self.compression_level is None:
                raise ValueError("lz4hc EROFS compression requires a level")
            if not 0 <= self.compression_level <= 12:
                raise ValueError("lz4hc EROFS compression level must be between 0 and 12")
        return self


class BuildConfig(BaseModel):
    """Top-level build settings from build.toml."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    materialize_network: Literal[BuildNetwork.DEFAULT]
    erofs: ErofsConfig = Field(default_factory=ErofsConfig)
    kernel: KernelConfig
    asset_dependencies: AssetDependencyConfig
    guest_rust_builder: GuestRustBuilderConfig
    asset_tools: AssetToolsConfig
    architectures: dict[str, ArchConfig]
    version_commands: dict[str, str] = Field(default_factory=dict)

    @model_validator(mode="after")
    def _architectures_non_empty(self):
        if not self.architectures:
            raise ValueError("architectures must have at least one entry")
        if set(self.asset_tools.architectures) != set(self.architectures):
            raise ValueError("asset tool architectures must exactly match build architectures")
        if set(self.asset_dependencies.architectures) != set(self.architectures):
            raise ValueError(
                "asset dependency architectures must exactly match build architectures"
            )
        for name, arch in self.architectures.items():
            node = self.asset_dependencies.architectures[name].node
            if int(node.version.partition(".")[0]) != arch.node_major:
                raise ValueError(
                    f"asset dependency Node major for {name} must match architecture node_major"
                )
        return self


# ---------------------------------------------------------------------------
# Package set configuration
# ---------------------------------------------------------------------------


class PackageNetworkConfig(BaseModel):
    """Network config for a package registry."""

    model_config = ConfigDict(frozen=True)

    name: str
    domains: list[str]
    allow_get: bool = True


class PackageSetConfig(BaseModel):
    """Package set definition from packages/{manager}.toml."""

    model_config = ConfigDict(frozen=True)

    name: str
    manager: PackageManager
    install_cmd: str
    packages: list[str]
    version_commands: dict[str, str] = Field(default_factory=dict)
    network: PackageNetworkConfig | None = None

    @model_validator(mode="after")
    def _validate_non_empty(self):
        if not self.packages:
            raise ValueError("packages must have at least one entry")
        if not self.install_cmd:
            raise ValueError("install_cmd must not be empty")
        bad = set(self.version_commands) - set(self.packages)
        if bad:
            raise ValueError(f"version_commands keys not in packages: {sorted(bad)}")
        return self


# ---------------------------------------------------------------------------
# MCP server configuration
# ---------------------------------------------------------------------------


class McpServerConfig(BaseModel):
    """MCP server definition from mcp/{server}.toml."""

    model_config = ConfigDict(frozen=True)

    name: str
    description: str = ""
    transport: McpTransport
    command: str | None = None
    url: str | None = None
    args: list[str] = Field(default_factory=list)
    env: dict[str, str] = Field(default_factory=dict)
    headers: dict[str, str] = Field(default_factory=dict)
    builtin: bool = False
    enabled: bool = True

    @model_validator(mode="after")
    def _validate_transport(self):
        if self.transport == McpTransport.STDIO and not self.command:
            raise ValueError("stdio transport requires 'command'")
        if self.transport == McpTransport.SSE and not self.url:
            raise ValueError("sse transport requires 'url'")
        return self


# ---------------------------------------------------------------------------
# Web security configuration
# ---------------------------------------------------------------------------


class WebServiceConfig(BaseModel):
    """A web service entry (search engine, registry, repository)."""

    model_config = ConfigDict(frozen=True)

    name: str
    enabled: bool = True
    domains: list[str]
    allow_get: bool = False
    allow_post: bool = False


class WebSecurityConfig(BaseModel):
    """Web security config from security/web.toml."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    http_upstream_ports: list[int] = Field(default_factory=lambda: [80, 3128, 3713, 8080, 11434])
    search: dict[str, WebServiceConfig] = Field(default_factory=dict)
    registry: dict[str, WebServiceConfig] = Field(default_factory=dict)
    repository: dict[str, WebServiceConfig] = Field(default_factory=dict)


# ---------------------------------------------------------------------------
# VM configuration
# ---------------------------------------------------------------------------


class VmResourcesConfig(BaseModel):
    """VM resource settings from vm/resources.toml."""

    model_config = ConfigDict(frozen=True)

    cpu_count: int = Field(default=4, ge=1, le=8)
    ram_gb: int = Field(default=4, ge=1, le=16)
    scratch_disk_size_gb: int = Field(default=16, ge=1, le=128)
    log_bodies: bool = False
    max_body_capture: int = Field(default=4096, ge=0, le=1048576)
    retention_days: int = Field(default=30, ge=1, le=365)
    max_sessions: int = Field(default=100, ge=1, le=10000)
    min_content_sessions: int = Field(default=25, ge=0, le=1000)
    max_disk_gb: int = Field(default=100, ge=1, le=1000)
    terminated_retention_days: int = Field(default=365, ge=30, le=3650)


class ShellFileConfig(BaseModel):
    """A shell config file (bashrc, tmux.conf)."""

    model_config = ConfigDict(frozen=True)

    path: str
    content: str


class ShellConfig(BaseModel):
    """Shell environment settings."""

    model_config = ConfigDict(frozen=True)

    term: str = "xterm-256color"
    home: str = "/root"
    path: str = "/opt/ai-clis/bin:/root/.local/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
    lang: str = "C"
    bashrc: ShellFileConfig | None = None
    tmux_conf: ShellFileConfig | None = None


class TlsConfig(BaseModel):
    """TLS configuration."""

    model_config = ConfigDict(frozen=True)

    ca_bundle: str = "/etc/ssl/certs/ca-certificates.crt"


class VmEnvironmentConfig(BaseModel):
    """VM environment config from vm/environment.toml."""

    model_config = ConfigDict(frozen=True)

    shell: ShellConfig = Field(default_factory=ShellConfig)
    tls: TlsConfig = Field(default_factory=TlsConfig)


# ---------------------------------------------------------------------------
# Image manifest (identity + changelog)
# ---------------------------------------------------------------------------


class ChangelogEntry(BaseModel):
    """Single changelog entry for an image version."""

    model_config = ConfigDict(frozen=True)

    version: str
    date: str
    changes: list[str]


class ImageManifestConfig(BaseModel):
    """Image identity and version history from manifest.toml."""

    model_config = ConfigDict(frozen=True)

    name: str
    version: str = "0.1.0"
    description: str = ""
    changelog: list[ChangelogEntry] = Field(default_factory=list)


# ---------------------------------------------------------------------------
# Top-level guest image config
# ---------------------------------------------------------------------------


class GuestImageConfig(BaseModel):
    """Top-level config combining the generated backend image workspace.

    Produced by load_guest_config() after capsem-admin materializes a profile.
    """

    model_config = ConfigDict(frozen=True)

    build: BuildConfig
    manifest: ImageManifestConfig | None = None
    guest_dir_path: str | None = None
    package_sets: dict[str, PackageSetConfig] = Field(default_factory=dict)
    mcp_servers: dict[str, McpServerConfig] = Field(default_factory=dict)
    web_security: WebSecurityConfig = Field(default_factory=WebSecurityConfig)
    vm_resources: VmResourcesConfig = Field(default_factory=VmResourcesConfig)
    vm_environment: VmEnvironmentConfig = Field(default_factory=VmEnvironmentConfig)
    profile_root_seed: bool = False
    profile_root_seed_path: str | None = None
    profile_build_script: bool = False
    profile_build_script_path: str | None = None
