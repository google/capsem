"""Capsem build configuration models -- Pydantic models for guest image config.

These models define the structure of the TOML config files in guest/config/.
Distinct from schema.py which defines the settings interchange format.
"""

from __future__ import annotations

from enum import Enum

from pydantic import BaseModel, ConfigDict, Field, model_validator

from capsem.builder.schema import McpTransport


# ---------------------------------------------------------------------------
# Enums
# ---------------------------------------------------------------------------


class Compression(str, Enum):
    """Compression algorithm for squashfs rootfs."""

    ZSTD = "zstd"
    GZIP = "gzip"
    LZO = "lzo"
    XZ = "xz"


class PackageManager(str, Enum):
    """Package manager for installing packages."""

    APT = "apt"
    UV = "uv"
    PIP = "pip"
    NPM = "npm"


# ---------------------------------------------------------------------------
# Build configuration
# ---------------------------------------------------------------------------


class ArchConfig(BaseModel):
    """Per-architecture build settings."""

    model_config = ConfigDict(frozen=True)

    base_image: str = "debian:bookworm-slim"
    docker_platform: str
    rust_target: str
    kernel_branch: str = "6.6"
    kernel_image: str
    defconfig: str
    node_major: int = 24


class BuildConfig(BaseModel):
    """Top-level build settings from build.toml."""

    model_config = ConfigDict(frozen=True)

    compression: Compression = Compression.ZSTD
    compression_level: int = Field(default=15, ge=1, le=22)
    architectures: dict[str, ArchConfig]

    @model_validator(mode="after")
    def _architectures_non_empty(self):
        if not self.architectures:
            raise ValueError("architectures must have at least one entry")
        return self


# ---------------------------------------------------------------------------
# AI provider configuration
# ---------------------------------------------------------------------------


class ApiKeyConfig(BaseModel):
    """API key definition for an AI provider."""

    model_config = ConfigDict(frozen=True)

    name: str
    env_vars: list[str]
    prefix: str = ""
    docs_url: str | None = None

    @model_validator(mode="after")
    def _env_vars_non_empty(self):
        if not self.env_vars:
            raise ValueError("env_vars must have at least one entry")
        return self


class NetworkConfig(BaseModel):
    """Network access config for an AI provider or package set."""

    model_config = ConfigDict(frozen=True)

    domains: list[str]
    allow_get: bool = False
    allow_post: bool = False

    @model_validator(mode="after")
    def _domains_non_empty(self):
        if not self.domains:
            raise ValueError("domains must have at least one entry")
        return self


class InstallConfig(BaseModel):
    """Installation config for an AI provider's CLI tools."""

    model_config = ConfigDict(frozen=True)

    manager: PackageManager
    prefix: str = ""
    packages: list[str]


class FileConfig(BaseModel):
    """A file to write to the guest filesystem."""

    model_config = ConfigDict(frozen=True)

    path: str
    content: str


class CliToolConfig(BaseModel):
    """CLI tool sub-group for an AI provider (e.g., Claude Code, Gemini CLI)."""

    model_config = ConfigDict(frozen=True)

    key: str
    name: str
    description: str = ""


class AiProviderConfig(BaseModel):
    """AI provider definition from ai/{provider}.toml."""

    model_config = ConfigDict(frozen=True)

    name: str
    description: str = ""
    enabled: bool = True
    api_key: ApiKeyConfig
    network: NetworkConfig
    install: InstallConfig | None = None
    cli: CliToolConfig | None = None
    files: dict[str, FileConfig] = Field(default_factory=dict)


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
    network: PackageNetworkConfig | None = None

    @model_validator(mode="after")
    def _validate_non_empty(self):
        if not self.packages:
            raise ValueError("packages must have at least one entry")
        if not self.install_cmd:
            raise ValueError("install_cmd must not be empty")
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

    model_config = ConfigDict(frozen=True)

    allow_read: bool = False
    allow_write: bool = False
    custom_allow: list[str] = Field(default_factory=list)
    custom_block: list[str] = Field(default_factory=list)
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
    """Top-level config combining all TOML files for a guest image.

    Produced by load_guest_config() which walks a guest/config/ directory.
    """

    model_config = ConfigDict(frozen=True)

    build: BuildConfig
    manifest: ImageManifestConfig | None = None
    ai_providers: dict[str, AiProviderConfig] = Field(default_factory=dict)
    package_sets: dict[str, PackageSetConfig] = Field(default_factory=dict)
    mcp_servers: dict[str, McpServerConfig] = Field(default_factory=dict)
    web_security: WebSecurityConfig = Field(default_factory=WebSecurityConfig)
    vm_resources: VmResourcesConfig = Field(default_factory=VmResourcesConfig)
    vm_environment: VmEnvironmentConfig = Field(default_factory=VmEnvironmentConfig)
