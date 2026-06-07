"""Typed Profile V2 service-settings contract for admin tooling.

Service settings are service/app-scoped control plane configuration. Profiles
remain the VM/session-scoped policy and package contract.

JSON boundaries intentionally go through Pydantic only:
``model_validate_json`` / ``TypeAdapter.validate_json`` for input and
``model_dump_json`` / ``TypeAdapter.dump_json`` for output. TOML is parsed once,
encoded through Pydantic's JSON serializer, then validated as the same payload.
"""

from __future__ import annotations

from enum import Enum
import os
from pathlib import Path
from typing import Annotated, Any, Literal
import re
import tomllib

import tomli_w
from pydantic import AnyUrl, BaseModel, ConfigDict, Field, TypeAdapter, model_validator


_PROFILE_ID_RE = re.compile(r"^[a-z0-9][a-z0-9-]*$")
_CONFIG_ID_RE = re.compile(r"^[a-z0-9][a-z0-9_.-]*$")

_RawTomlAdapter = TypeAdapter(dict[str, Any])

NonEmptyStr = Annotated[str, Field(min_length=1)]
PathStr = Annotated[str, Field(min_length=1)]
ProfileId = Annotated[str, Field(pattern=_PROFILE_ID_RE.pattern)]
ConfigId = Annotated[str, Field(pattern=_CONFIG_ID_RE.pattern)]


def _default_capsem_home() -> str:
    capsem_home = os.environ.get("CAPSEM_HOME")
    if capsem_home:
        return capsem_home
    home = os.environ.get("HOME")
    if home:
        return str(Path(home) / ".capsem")
    return ".capsem"


def _default_user_profile_dirs() -> list[str]:
    return [str(Path(_default_capsem_home()) / "profiles")]


def _default_base_profile_dirs() -> list[str]:
    return [str(Path(_default_capsem_home()) / "profiles" / "base")]


class StrictModel(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)


class Theme(str, Enum):
    SYSTEM = "system"
    LIGHT = "light"
    DARK = "dark"


class CredentialBackend(str, Enum):
    TOML = "toml"
    KEYCHAIN = "keychain"


class TelemetryFailureMode(str, Enum):
    DROP = "drop"
    DISABLE = "disable"
    BACKPRESSURE = "backpressure"


class RemotePolicyFailureMode(str, Enum):
    FAIL_OPEN = "fail-open"
    FAIL_CLOSED = "fail-closed"


class CorpDirectiveOperation(str, Enum):
    ADD = "add"
    REMOVE = "remove"
    REPLACE = "replace"
    LOCK = "lock"
    FORBID = "forbid"


class AppearanceSettings(StrictModel):
    theme: Theme = Theme.SYSTEM
    accent: NonEmptyStr = "blue"


class AppSettings(StrictModel):
    auto_launch: bool = True
    appearance: AppearanceSettings = Field(default_factory=AppearanceSettings)
    google_config_path: PathStr | None = None


class ProfileRootSettings(StrictModel):
    base_dirs: list[PathStr] = Field(default_factory=_default_base_profile_dirs)
    corp_dirs: list[PathStr] = Field(default_factory=list)
    user_dirs: list[PathStr] = Field(default_factory=_default_user_profile_dirs)
    default_profile: ProfileId = "everyday-work"
    allow_user_profiles: bool = True
    allow_user_fork: bool = True
    allow_user_delete: bool = True

    @model_validator(mode="after")
    def _base_dirs_required(self) -> "ProfileRootSettings":
        if not self.base_dirs:
            raise ValueError("profiles.base_dirs requires at least one directory")
        return self


class AssetLocationSettings(StrictModel):
    assets_dir: PathStr | None = None
    image_roots: list[PathStr] = Field(default_factory=list)
    download_base_url: AnyUrl | None = None


class TomlCredential(StrictModel):
    description: NonEmptyStr | None = None
    value: NonEmptyStr


class CredentialSettings(StrictModel):
    backend: CredentialBackend = CredentialBackend.TOML
    items: dict[ConfigId, TomlCredential] = Field(default_factory=dict)


class TelemetrySettings(StrictModel):
    enabled: bool = False
    endpoint: AnyUrl | None = None
    headers: dict[NonEmptyStr, NonEmptyStr] = Field(default_factory=dict)
    batch_max_events: Annotated[int, Field(ge=1, le=65535)] = 128
    flush_interval_ms: Annotated[int, Field(ge=1)] = 5000
    redact_secrets: bool = True
    retry_attempts: Annotated[int, Field(ge=0, le=255)] = 3
    failure_mode: TelemetryFailureMode = TelemetryFailureMode.DROP

    @model_validator(mode="after")
    def _endpoint_required_when_enabled(self) -> "TelemetrySettings":
        if self.enabled and self.endpoint is None:
            raise ValueError("telemetry.endpoint is required when telemetry is enabled")
        return self


class RemotePolicySettings(StrictModel):
    enabled: bool = False
    endpoint: AnyUrl | None = None
    auth_token: NonEmptyStr | None = None
    timeout_ms: Annotated[int, Field(ge=100, le=60000)] = 1500
    failure_mode: RemotePolicyFailureMode = RemotePolicyFailureMode.FAIL_CLOSED

    @model_validator(mode="after")
    def _endpoint_required_when_enabled(self) -> "RemotePolicySettings":
        if self.enabled and self.endpoint is None:
            raise ValueError("remote_policy.endpoint is required when remote policy is enabled")
        return self


class ProfileCatalogSettings(StrictModel):
    manifest_url: AnyUrl | None = None
    profile_payload_pubkey: NonEmptyStr | None = None
    check_interval_secs: Annotated[int, Field(ge=60)] = 21600

    @model_validator(mode="after")
    def _catalog_source_is_complete(self) -> "ProfileCatalogSettings":
        if (self.manifest_url is None) != (self.profile_payload_pubkey is None):
            raise ValueError(
                "profile_catalog.manifest_url and profile_payload_pubkey must be set together"
            )
        if self.manifest_url is not None:
            scheme = self.manifest_url.scheme
            host = self.manifest_url.host or ""
            if scheme == "http" and host not in {"localhost", "127.0.0.1", "::1"}:
                raise ValueError("profile_catalog.manifest_url only allows http for loopback")
        return self


class CorpDirective(StrictModel):
    operation: CorpDirectiveOperation
    path: NonEmptyStr
    value: Any | None = None
    reason: NonEmptyStr | None = None

    @model_validator(mode="after")
    def _value_matches_operation(self) -> "CorpDirective":
        needs_value = self.operation in {
            CorpDirectiveOperation.ADD,
            CorpDirectiveOperation.REPLACE,
            CorpDirectiveOperation.LOCK,
        }
        if needs_value and self.value is None:
            raise ValueError("add/replace/lock directives require value")
        if not needs_value and self.value is not None:
            raise ValueError("remove/forbid directives must not carry value")
        return self


class ServiceSettingsV2(StrictModel):
    version: Literal[1] = 1
    app: AppSettings = Field(default_factory=AppSettings)
    profiles: ProfileRootSettings = Field(default_factory=ProfileRootSettings)
    assets: AssetLocationSettings = Field(default_factory=AssetLocationSettings)
    credentials: CredentialSettings = Field(default_factory=CredentialSettings)
    telemetry: TelemetrySettings = Field(default_factory=TelemetrySettings)
    remote_policy: RemotePolicySettings = Field(default_factory=RemotePolicySettings)
    profile_catalog: ProfileCatalogSettings = Field(default_factory=ProfileCatalogSettings)
    corp_directives: list[CorpDirective] = Field(default_factory=list)


def validate_service_settings_json(payload: str | bytes) -> ServiceSettingsV2:
    return ServiceSettingsV2.model_validate_json(payload)


def dump_service_settings_json(settings: ServiceSettingsV2) -> str:
    return settings.model_dump_json(by_alias=True, exclude_none=True, indent=2)


def dump_service_settings_toml(settings: ServiceSettingsV2) -> str:
    return tomli_w.dumps(
        settings.model_dump(mode="json", by_alias=True, exclude_none=True)
    )


def create_service_settings_draft(
    *,
    default_profile: str = "everyday-work",
    base_dirs: list[str] | None = None,
    corp_dirs: list[str] | None = None,
    user_dirs: list[str] | None = None,
    assets_dir: str | None = None,
) -> ServiceSettingsV2:
    return ServiceSettingsV2(
        profiles=ProfileRootSettings(
            base_dirs=base_dirs or _default_base_profile_dirs(),
            corp_dirs=corp_dirs or [],
            user_dirs=user_dirs or _default_user_profile_dirs(),
            default_profile=default_profile,
        ),
        assets=AssetLocationSettings(assets_dir=assets_dir),
    )


def validate_service_settings_toml(path: Path) -> ServiceSettingsV2:
    parsed = tomllib.loads(path.read_text(encoding="utf-8"))
    payload = _RawTomlAdapter.dump_json(parsed)
    return ServiceSettingsV2.model_validate_json(payload)


def dump_service_settings_schema_json() -> str:
    schema = TypeAdapter(ServiceSettingsV2).json_schema(
        by_alias=True,
        ref_template="#/$defs/{model}",
    )
    return _RawTomlAdapter.dump_json(schema, indent=2).decode()
