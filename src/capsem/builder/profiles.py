"""Typed Profile V2 and manifest contracts for admin tooling.

JSON boundaries intentionally go through Pydantic only:
``model_validate_json`` / ``TypeAdapter.validate_json`` for input and
``model_dump_json`` / ``TypeAdapter.dump_json`` for output. TOML is parsed once,
encoded through Pydantic's JSON serializer, then validated as the same payload.
"""

from __future__ import annotations

from datetime import date
from enum import Enum
from pathlib import Path
from typing import Annotated, Any, Literal
import re
import tomllib

import blake3
import tomli_w
from pydantic import (
    AnyUrl,
    BaseModel,
    ConfigDict,
    Field,
    TypeAdapter,
    field_validator,
    model_validator,
)

from capsem.builder.models import GuestImageConfig, PackageManager


_PROFILE_ID_RE = re.compile(r"^[a-z0-9][a-z0-9-]{2,63}$")
_REVISION_RE = re.compile(r"^[0-9]{4}\.[0-9]{4}\.[0-9]+$")
_BLAKE3_RE = re.compile(r"^blake3:[0-9a-f]{64}$")

_RawTomlAdapter = TypeAdapter(dict[str, Any])

NonEmptyStr = Annotated[str, Field(min_length=1)]
VersionStr = Annotated[str, Field(min_length=1)]
ProfileId = Annotated[str, Field(pattern=_PROFILE_ID_RE.pattern)]
Revision = Annotated[str, Field(pattern=_REVISION_RE.pattern)]
Blake3Hash = Annotated[str, Field(pattern=_BLAKE3_RE.pattern)]


class StrictModel(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)


class ProfileRevisionStatus(str, Enum):
    ACTIVE = "active"
    DEPRECATED = "deprecated"
    REVOKED = "revoked"

    def can_be_current(self) -> bool:
        return self is ProfileRevisionStatus.ACTIVE

    def allows_install_or_update(self) -> bool:
        return self is ProfileRevisionStatus.ACTIVE

    def allows_new_vm(self) -> bool:
        return self is ProfileRevisionStatus.ACTIVE

    def allows_existing_vm(self) -> bool:
        return self in {
            ProfileRevisionStatus.ACTIVE,
            ProfileRevisionStatus.DEPRECATED,
        }


class ProfileType(str, Enum):
    EVERYDAY_WORK = "everyday-work"
    CODING = "coding"


class ProfileUi(str, Enum):
    EVERYDAY = "everyday"
    CODING = "coding"


class VmNetworkMode(str, Enum):
    PROXIED = "proxied"
    DISABLED = "disabled"
    DIRECT = "direct"


class ToolSource(str, Enum):
    GUEST = "guest"
    HOST = "host"
    PROFILE = "profile"


class CapabilityMode(str, Enum):
    ALLOW = "allow"
    ASK = "ask"
    BLOCK = "block"
    AUDIT = "audit"


class RuleDecision(str, Enum):
    ALLOW = "allow"
    ASK = "ask"
    BLOCK = "block"
    REWRITE = "rewrite"


class Compatibility(StrictModel):
    min_binary: VersionStr
    max_binary: str = ""
    guest_abi: Annotated[str, Field(pattern=r"^capsem-guest-v[0-9]+$")]


class GeneralSettings(StrictModel):
    display_name: NonEmptyStr | None = None


class AppearanceSettings(StrictModel):
    theme: Literal["inherit-service", "system", "light", "dark"] | None = None
    accent: str | None = None


class ProfileSectionEditability(StrictModel):
    general: bool = True
    appearance: bool = True
    ai: bool = True
    mcp_servers: bool = Field(default=True, alias="mcpServers")
    skills: bool = True
    packages: bool = True
    tools: bool = True
    vm: bool = True
    security_capabilities: bool = True
    security_rules: bool = True


class SecurityRule(StrictModel):
    callback: NonEmptyStr = Field(alias="on")
    condition: NonEmptyStr = Field(alias="if")
    decision: RuleDecision
    priority: Annotated[int, Field(ge=-1000, le=999)] = 1
    rewrite_target: NonEmptyStr | None = None
    rewrite_value: NonEmptyStr | None = None
    strip_request_headers: list[NonEmptyStr] = Field(default_factory=list)
    strip_response_headers: list[NonEmptyStr] = Field(default_factory=list)
    reason: NonEmptyStr | None = None


class SecurityRules(StrictModel):
    mcp: dict[str, SecurityRule] = Field(default_factory=dict)
    http: dict[str, SecurityRule] = Field(default_factory=dict)
    dns: dict[str, SecurityRule] = Field(default_factory=dict)
    model: dict[str, SecurityRule] = Field(default_factory=dict)
    hook: dict[str, SecurityRule] = Field(default_factory=dict)


class SecurityCapabilities(StrictModel):
    credential_brokerage: CapabilityMode | None = None
    pii_detection: CapabilityMode | None = None
    mcp_rag: CapabilityMode | None = None
    mcp_tools: CapabilityMode | None = None
    network_egress: CapabilityMode | None = None
    file_boundaries: CapabilityMode | None = None
    audit: CapabilityMode | None = None


class SecuritySettings(StrictModel):
    capabilities: SecurityCapabilities = Field(default_factory=SecurityCapabilities)
    rules: SecurityRules = Field(default_factory=SecurityRules)


class AiProvider(StrictModel):
    enabled: bool | None = None
    model: NonEmptyStr | None = None
    base_url: AnyUrl | None = None
    credential_refs: list[NonEmptyStr] = Field(default_factory=list)
    rules: SecurityRules = Field(default_factory=SecurityRules)


class AiSettings(StrictModel):
    providers: dict[str, AiProvider] = Field(default_factory=dict)


class McpServerCapsem(StrictModel):
    credential_refs: list[NonEmptyStr] = Field(default_factory=list)
    allowed_tools: list[NonEmptyStr] = Field(default_factory=list)
    rules: SecurityRules = Field(default_factory=SecurityRules)


class McpServer(StrictModel):
    enabled: bool = True
    type_: Literal["stdio", "http", "sse"] | None = Field(default=None, alias="type")
    command: NonEmptyStr | None = None
    args: list[NonEmptyStr] = Field(default_factory=list)
    env: dict[NonEmptyStr, str] = Field(default_factory=dict)
    url: AnyUrl | None = None
    headers: dict[NonEmptyStr, str] = Field(default_factory=dict)
    bearer_token: str | None = Field(default=None, alias="bearerToken")
    pool_size: Annotated[int, Field(ge=1)] | None = None
    pool_safe_tools: list[NonEmptyStr] = Field(default_factory=list)
    capsem: McpServerCapsem = Field(default_factory=McpServerCapsem)

    @model_validator(mode="after")
    def _transport_is_standard_and_complete(self) -> "McpServer":
        has_command = self.command is not None
        has_url = self.url is not None
        if has_command == has_url:
            raise ValueError("MCP server must set exactly one of command or url")
        if self.type_ == "stdio" and not has_command:
            raise ValueError("MCP server type=stdio requires command")
        if self.type_ in {"http", "sse"} and not has_url:
            raise ValueError("MCP server type=http/sse requires url")
        return self


class SkillsSettings(StrictModel):
    groups: list[NonEmptyStr] = Field(default_factory=list)
    enabled: list[NonEmptyStr] = Field(default_factory=list)
    disabled: list[NonEmptyStr] = Field(default_factory=list)


class AssetDeclaration(StrictModel):
    url: AnyUrl
    hash: Blake3Hash
    signature_url: AnyUrl
    size: Annotated[int, Field(ge=1)]
    content_type: NonEmptyStr


class ArchAssets(StrictModel):
    kernel: AssetDeclaration
    initrd: AssetDeclaration
    rootfs: AssetDeclaration


class VmSettings(StrictModel):
    memory_mib: Annotated[int, Field(ge=512)]
    cpus: Annotated[int, Field(ge=1)]
    disk_mib: Annotated[int, Field(ge=1024)]
    network: VmNetworkMode
    track_rootfs_dependencies: bool = True
    rootfs_image: NonEmptyStr | None = None
    assets: dict[Literal["arm64", "x86_64"], ArchAssets]


class SystemPackages(StrictModel):
    distro: Literal["debian"]
    release: NonEmptyStr
    apt: dict[str, VersionStr] = Field(default_factory=dict)


class PackageContract(StrictModel):
    runtimes: dict[str, VersionStr]
    python_modules: dict[str, VersionStr] = Field(default_factory=dict)
    node_packages: dict[str, VersionStr] = Field(default_factory=dict)
    system: SystemPackages


class ToolContract(StrictModel):
    version: VersionStr
    required: bool
    source: ToolSource


class ProfilePayloadV2(StrictModel):
    schema_: Literal["capsem.profile.v2"] = Field(alias="schema")
    version: Literal[2]
    id: ProfileId
    revision: Revision
    name: NonEmptyStr
    description: NonEmptyStr
    best_for: NonEmptyStr
    profile_type: ProfileType
    ui: ProfileUi
    icon_svg: str | None = None
    extends_profile_id: ProfileId | None = None
    extends_profile_revision: Revision | None = None
    compatibility: Compatibility
    general: GeneralSettings = Field(default_factory=GeneralSettings)
    appearance: AppearanceSettings = Field(default_factory=AppearanceSettings)
    editable: ProfileSectionEditability = Field(default_factory=ProfileSectionEditability)
    ai: AiSettings = Field(default_factory=AiSettings)
    mcp_servers: dict[str, McpServer] = Field(default_factory=dict, alias="mcpServers")
    skills: SkillsSettings = Field(default_factory=SkillsSettings)
    vm: VmSettings
    packages: PackageContract
    tools: dict[str, ToolContract]
    security: SecuritySettings

    @model_validator(mode="after")
    def _parent_revision_pair(self) -> "ProfilePayloadV2":
        if (self.extends_profile_id is None) != (self.extends_profile_revision is None):
            raise ValueError(
                "extends_profile_id and extends_profile_revision must be set together"
            )
        return self

    @field_validator("icon_svg")
    @classmethod
    def _icon_svg_is_inline_svg(cls, value: str | None) -> str | None:
        if value is not None and not value.lstrip().startswith("<svg"):
            raise ValueError("icon_svg must be inline SVG")
        return value


class ManifestProfileRevision(StrictModel):
    status: ProfileRevisionStatus
    min_binary: VersionStr
    max_binary: str | None = None
    profile_url: AnyUrl
    profile_hash: Blake3Hash
    profile_signature_url: AnyUrl


class ManifestProfile(StrictModel):
    current_revision: Revision
    revisions: dict[Revision, ManifestProfileRevision]

    @model_validator(mode="after")
    def _current_revision_is_active(self) -> "ManifestProfile":
        current = self.revisions.get(self.current_revision)
        if current is None:
            raise ValueError("current_revision must exist in revisions")
        if current.status is not ProfileRevisionStatus.ACTIVE:
            raise ValueError("current_revision must be active")
        return self


class ProfileManifest(StrictModel):
    format: Literal[1]
    profiles: dict[ProfileId, ManifestProfile]

    def current_revision(self, profile_id: str) -> "ResolvedProfileRevision":
        profile = self.profiles.get(profile_id)
        if profile is None:
            raise KeyError(f"profile '{profile_id}' not found")
        record = profile.revisions.get(profile.current_revision)
        if record is None:
            raise KeyError(
                f"current revision '{profile.current_revision}' "
                f"for profile '{profile_id}' not found"
            )
        return ResolvedProfileRevision(
            profile_id=profile_id,
            revision=profile.current_revision,
            record=record,
        )

    def revision(self, profile_id: str, revision: str) -> "ResolvedProfileRevision":
        profile = self.profiles.get(profile_id)
        if profile is None:
            raise KeyError(f"profile '{profile_id}' not found")
        record = profile.revisions.get(revision)
        if record is None:
            raise KeyError(f"revision '{revision}' for profile '{profile_id}' not found")
        return ResolvedProfileRevision(
            profile_id=profile_id,
            revision=revision,
            record=record,
        )


class ResolvedProfileRevision(StrictModel):
    profile_id: ProfileId
    revision: Revision
    record: ManifestProfileRevision


class VerifiedProfilePayload(StrictModel):
    profile: ProfilePayloadV2
    payload_hash: Blake3Hash


ProfilePayloadV2Adapter = TypeAdapter(ProfilePayloadV2)
ProfileManifestAdapter = TypeAdapter(ProfileManifest)


def validate_profile_json(payload: str | bytes) -> ProfilePayloadV2:
    return ProfilePayloadV2.model_validate_json(payload)


def dump_profile_json(profile: ProfilePayloadV2) -> str:
    return profile.model_dump_json(by_alias=True, exclude_none=True, indent=2)


def dump_profile_toml(profile: ProfilePayloadV2) -> str:
    return tomli_w.dumps(
        profile.model_dump(mode="json", by_alias=True, exclude_none=True)
    )


def dump_profile_schema_json() -> str:
    schema = TypeAdapter(ProfilePayloadV2).json_schema(
        by_alias=True,
        ref_template="#/$defs/{model}",
    )
    return _RawTomlAdapter.dump_json(schema, indent=2).decode()


def default_profile_revision(today: date | None = None) -> str:
    value = today or date.today()
    return f"{value:%Y.%m%d}.1"


def _placeholder_asset(
    profile_id: str,
    revision: str,
    arch: Literal["arm64", "x86_64"],
    kind: Literal["kernel", "initrd", "rootfs"],
    hash_hex: str,
    size: int,
    content_type: str,
) -> AssetDeclaration:
    return AssetDeclaration(
        url=(
            "https://assets.example.invalid/capsem/profiles/"
            f"{profile_id}/{revision}/{arch}/{kind}"
        ),
        hash=f"blake3:{hash_hex}",
        signature_url=(
            "https://assets.example.invalid/capsem/profiles/"
            f"{profile_id}/{revision}/{arch}/{kind}.minisig"
        ),
        size=size,
        content_type=content_type,
    )


def create_profile_draft(
    profile_id: str,
    *,
    revision: str | None = None,
    name: str | None = None,
    description: str | None = None,
    best_for: str | None = None,
    profile_type: ProfileType = ProfileType.CODING,
    ui: ProfileUi | None = None,
) -> ProfilePayloadV2:
    resolved_revision = revision or default_profile_revision()
    resolved_name = name or profile_id.replace("-", " ").title()
    resolved_description = description or f"{resolved_name} Profile V2 draft."
    resolved_best_for = best_for or "Profile-backed Capsem workspaces."
    resolved_ui = ui or (
        ProfileUi.EVERYDAY
        if profile_type is ProfileType.EVERYDAY_WORK
        else ProfileUi.CODING
    )

    return ProfilePayloadV2(
        schema="capsem.profile.v2",
        version=2,
        id=profile_id,
        revision=resolved_revision,
        name=resolved_name,
        description=resolved_description,
        best_for=resolved_best_for,
        profile_type=profile_type,
        ui=resolved_ui,
        compatibility=Compatibility(
            min_binary="1.0.0",
            guest_abi="capsem-guest-v2",
        ),
        editable=ProfileSectionEditability(),
        vm=VmSettings(
            memory_mib=8192,
            cpus=4,
            disk_mib=32768,
            network=VmNetworkMode.PROXIED,
            assets={
                "arm64": ArchAssets(
                    kernel=_placeholder_asset(
                        profile_id,
                        resolved_revision,
                        "arm64",
                        "kernel",
                        "a" * 64,
                        1,
                        "application/octet-stream",
                    ),
                    initrd=_placeholder_asset(
                        profile_id,
                        resolved_revision,
                        "arm64",
                        "initrd",
                        "b" * 64,
                        1,
                        "application/octet-stream",
                    ),
                    rootfs=_placeholder_asset(
                        profile_id,
                        resolved_revision,
                        "arm64",
                        "rootfs",
                        "c" * 64,
                        1,
                        "application/vnd.squashfs",
                    ),
                ),
                "x86_64": ArchAssets(
                    kernel=_placeholder_asset(
                        profile_id,
                        resolved_revision,
                        "x86_64",
                        "kernel",
                        "d" * 64,
                        1,
                        "application/octet-stream",
                    ),
                    initrd=_placeholder_asset(
                        profile_id,
                        resolved_revision,
                        "x86_64",
                        "initrd",
                        "e" * 64,
                        1,
                        "application/octet-stream",
                    ),
                    rootfs=_placeholder_asset(
                        profile_id,
                        resolved_revision,
                        "x86_64",
                        "rootfs",
                        "f" * 64,
                        1,
                        "application/vnd.squashfs",
                    ),
                ),
            },
        ),
        packages=PackageContract(
            runtimes={"python": "3.12"},
            system=SystemPackages(distro="debian", release="bookworm"),
        ),
        tools={
            "capsem_doctor": ToolContract(
                version="2026.05.20",
                required=True,
                source=ToolSource.GUEST,
            )
        },
        security=SecuritySettings(
            capabilities=SecurityCapabilities(
                credential_brokerage=CapabilityMode.ASK,
                network_egress=CapabilityMode.ASK,
                file_boundaries=CapabilityMode.AUDIT,
                audit=CapabilityMode.ALLOW,
            )
        ),
    )


def create_builtin_profile_drafts(
    *,
    revision: str | None = None,
    guest_config: GuestImageConfig | None = None,
) -> list[ProfilePayloadV2]:
    """Generate the built-in base profiles from the same typed contract."""
    resolved_revision = revision or default_profile_revision()
    if guest_config is not None:
        return [
            create_profile_from_guest_config(
                guest_config,
                "everyday-work",
                revision=resolved_revision,
                name="Everyday Work",
                description="Balanced defaults for daily work sessions.",
                best_for="Daily work with useful tools and measured security prompts.",
                profile_type=ProfileType.EVERYDAY_WORK,
                ui=ProfileUi.EVERYDAY,
            ),
            create_profile_from_guest_config(
                guest_config,
                "coding",
                revision=resolved_revision,
                name="Coding",
                description="Focused defaults for software development sessions.",
                best_for="Coding agents, repository work, tests, and developer tooling.",
                profile_type=ProfileType.CODING,
                ui=ProfileUi.CODING,
            ),
        ]
    return [
        create_profile_draft(
            "everyday-work",
            revision=resolved_revision,
            name="Everyday Work",
            description="Balanced defaults for daily work sessions.",
            best_for="Daily work with useful tools and measured security prompts.",
            profile_type=ProfileType.EVERYDAY_WORK,
            ui=ProfileUi.EVERYDAY,
        ),
        create_profile_draft(
            "coding",
            revision=resolved_revision,
            name="Coding",
            description="Focused defaults for software development sessions.",
            best_for="Coding agents, repository work, tests, and developer tooling.",
            profile_type=ProfileType.CODING,
            ui=ProfileUi.CODING,
        ),
    ]


_FLOATING_VERSION = "*"


def _split_versioned_package(spec: str, separator: str) -> tuple[str, str]:
    if separator in spec:
        name, version = spec.split(separator, 1)
        if name and version:
            return name, version
    return spec, _FLOATING_VERSION


def _split_npm_package(spec: str) -> tuple[str, str]:
    if spec.startswith("@"):
        rest = spec[1:]
        if "@" in rest:
            package, version = spec.rsplit("@", 1)
            if package and version:
                return package, version
        return spec, _FLOATING_VERSION
    return _split_versioned_package(spec, "@")


def create_profile_from_guest_config(
    guest_config: GuestImageConfig,
    profile_id: str,
    *,
    revision: str | None = None,
    name: str | None = None,
    description: str | None = None,
    best_for: str | None = None,
    profile_type: ProfileType = ProfileType.CODING,
    ui: ProfileUi | None = None,
) -> ProfilePayloadV2:
    """Create a Profile V2 payload from the existing guest image config.

    Floating package versions use ``*`` and are rendered back to unpinned
    package specs by the image workspace generator. The later SBOM/in-guest
    verification slice records resolved versions after the image is built.
    """
    resolved_revision = revision or default_profile_revision()
    draft = create_profile_draft(
        profile_id,
        revision=resolved_revision,
        name=name,
        description=description,
        best_for=best_for,
        profile_type=profile_type,
        ui=ui,
    )

    apt_packages: dict[str, str] = {}
    python_modules: dict[str, str] = {}
    node_packages: dict[str, str] = {}
    for package_set in guest_config.package_sets.values():
        if package_set.manager is PackageManager.APT:
            for spec in package_set.packages:
                package, version = _split_versioned_package(spec, "=")
                apt_packages[package] = version
        elif package_set.manager in {PackageManager.UV, PackageManager.PIP}:
            for spec in package_set.packages:
                package, version = _split_versioned_package(spec, "==")
                python_modules[package] = version
        elif package_set.manager is PackageManager.NPM:
            for spec in package_set.packages:
                package, version = _split_npm_package(spec)
                node_packages[package] = version

    tools: dict[str, ToolContract] = {
        "capsem_doctor": ToolContract(
            version="2026.05.20",
            required=True,
            source=ToolSource.GUEST,
        )
    }
    for provider in guest_config.ai_providers.values():
        install = provider.install
        if install is not None and install.manager is PackageManager.NPM:
            for spec in install.packages:
                package, version = _split_npm_package(spec)
                node_packages[package] = version
        if provider.cli is not None:
            tools[provider.cli.key] = ToolContract(
                version=_FLOATING_VERSION,
                required=provider.enabled,
                source=ToolSource.GUEST,
            )

    mcp_servers: dict[str, McpServer] = {}
    for server_id, server in guest_config.mcp_servers.items():
        mcp_servers[server_id] = McpServer(
            enabled=server.enabled,
            type=server.transport.value,
            command=server.command,
            args=server.args,
            env=server.env,
            url=server.url,
            headers=server.headers,
        )

    node_major = next(iter(guest_config.build.architectures.values())).node_major
    runtime_versions = {
        "python": "3.12",
        "node": str(node_major),
        "npm": _FLOATING_VERSION,
        "uv": _FLOATING_VERSION,
    }
    packages = PackageContract(
        runtimes=runtime_versions,
        python_modules=python_modules,
        node_packages=node_packages,
        system=SystemPackages(
            distro="debian",
            release="bookworm",
            apt=apt_packages,
        ),
    )
    vm = draft.vm.model_copy(
        update={
            "memory_mib": guest_config.vm_resources.ram_gb * 1024,
            "cpus": guest_config.vm_resources.cpu_count,
            "disk_mib": guest_config.vm_resources.scratch_disk_size_gb * 1024,
        }
    )

    return draft.model_copy(
        update={
            "ai": AiSettings(
                providers={
                    provider_id: AiProvider(
                        enabled=provider.enabled,
                        credential_refs=provider.api_key.env_vars,
                    )
                    for provider_id, provider in guest_config.ai_providers.items()
                }
            ),
            "mcp_servers": mcp_servers,
            "vm": vm,
            "packages": packages,
            "tools": tools,
        }
    )


def validate_manifest_json(payload: str | bytes) -> ProfileManifest:
    return ProfileManifest.model_validate_json(payload)


def dump_manifest_json(manifest: ProfileManifest) -> str:
    return manifest.model_dump_json(exclude_none=True, indent=2)


def verify_installable_profile_payload(
    revision: ResolvedProfileRevision,
    payload: str | bytes,
) -> VerifiedProfilePayload:
    if not revision.record.status.allows_install_or_update():
        raise ValueError(
            f"profile '{revision.profile_id}' revision '{revision.revision}' "
            f"has status '{revision.record.status.value}' "
            "and cannot be installed or updated"
        )

    payload_bytes = payload.encode() if isinstance(payload, str) else payload
    payload_hash = f"blake3:{blake3.blake3(payload_bytes).hexdigest()}"
    if payload_hash != revision.record.profile_hash:
        raise ValueError(
            f"profile payload hash mismatch for '{revision.profile_id}@{revision.revision}' "
            f"(expected {revision.record.profile_hash}, got {payload_hash})"
        )

    profile = ProfilePayloadV2.model_validate_json(payload_bytes)
    if profile.id != revision.profile_id:
        raise ValueError(
            f"profile payload id '{profile.id}' does not match "
            f"manifest profile '{revision.profile_id}'"
        )
    if profile.revision != revision.revision:
        raise ValueError(
            f"profile payload revision '{profile.revision}' does not match "
            f"manifest revision '{revision.revision}'"
        )

    return VerifiedProfilePayload(profile=profile, payload_hash=payload_hash)


def validate_profile_toml(path: Path) -> ProfilePayloadV2:
    with path.open("rb") as handle:
        parsed = tomllib.load(handle)
    payload = _RawTomlAdapter.dump_json(parsed)
    return ProfilePayloadV2.model_validate_json(payload)
