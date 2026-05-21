"""Typed policy and detection pack contracts for capsem-admin."""

from __future__ import annotations

from enum import Enum
from pathlib import Path
from typing import Annotated, Any, Literal
import re
import tomllib

import tomli_w
import yaml
from pydantic import BaseModel, ConfigDict, Field, TypeAdapter, model_validator


_PACK_ID_RE = re.compile(r"^[a-z0-9][a-z0-9_.-]{2,95}$")
_RULE_ID_RE = re.compile(r"^[a-z0-9][a-z0-9_.-]{1,127}$")
_RawTomlAdapter = TypeAdapter(dict[str, Any])

NonEmptyStr = Annotated[str, Field(min_length=1)]
PackId = Annotated[str, Field(pattern=_PACK_ID_RE.pattern)]
RuleId = Annotated[str, Field(pattern=_RULE_ID_RE.pattern)]


class StrictModel(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, populate_by_name=True)


class PackStatus(str, Enum):
    ACTIVE = "active"
    DEPRECATED = "deprecated"
    REVOKED = "revoked"


class PackOwner(str, Enum):
    CORP = "corp"
    VENDOR = "vendor"
    USER = "user"


class EventFamily(str, Enum):
    DNS = "dns"
    HTTP = "http"
    MCP = "mcp"
    MODEL = "model"
    FILE = "file"
    PROCESS = "process"
    CREDENTIAL = "credential"
    VM = "vm"
    PROFILE = "profile"
    CONVERSATION = "conversation"


class PolicyDecision(str, Enum):
    ALLOW = "allow"
    BLOCK = "block"
    ASK = "ask"
    REWRITE = "rewrite"


class Severity(str, Enum):
    INFO = "info"
    LOW = "low"
    MEDIUM = "medium"
    HIGH = "high"
    CRITICAL = "critical"


class Confidence(str, Enum):
    LOW = "low"
    MEDIUM = "medium"
    HIGH = "high"


class ProfileScope(StrictModel):
    profile_ids: list[NonEmptyStr] = Field(default_factory=list)
    profile_types: list[Literal["everyday-work", "coding"]] = Field(default_factory=list)
    required_tools: list[NonEmptyStr] = Field(default_factory=list)
    required_packages: list[NonEmptyStr] = Field(default_factory=list)


class PackLocks(StrictModel):
    editable: bool = True
    allow_user_disable: bool = True
    allow_severity_override: bool = True
    allow_suppression: bool = True


class RewritePayload(StrictModel):
    target: NonEmptyStr
    value: str | None = None
    strip_request_headers: list[NonEmptyStr] = Field(default_factory=list)
    strip_response_headers: list[NonEmptyStr] = Field(default_factory=list)


class RuleProvenance(StrictModel):
    generated_by: NonEmptyStr | None = None
    source_pack: NonEmptyStr | None = None
    source_profile_revision: NonEmptyStr | None = None
    confirm_id: NonEmptyStr | None = None
    detection_suggestion_id: NonEmptyStr | None = None


class PolicyRuleV1(StrictModel):
    id: RuleId
    name: NonEmptyStr
    description: NonEmptyStr | None = None
    enabled: bool = True
    event_family: EventFamily
    event_type: NonEmptyStr
    priority: Annotated[int, Field(ge=-1000, le=1000)] = 100
    condition: NonEmptyStr
    decision: PolicyDecision
    rewrite: RewritePayload | None = None
    reason: NonEmptyStr | None = None
    tags: list[NonEmptyStr] = Field(default_factory=list)
    references: list[NonEmptyStr] = Field(default_factory=list)
    provenance: RuleProvenance = Field(default_factory=RuleProvenance)

    @model_validator(mode="after")
    def _rewrite_matches_decision(self) -> "PolicyRuleV1":
        if self.decision is PolicyDecision.REWRITE and self.rewrite is None:
            raise ValueError("rewrite decision requires rewrite payload")
        if self.decision is not PolicyDecision.REWRITE and self.rewrite is not None:
            raise ValueError("rewrite payload is only valid for rewrite decision")
        return self


class PolicyPackV1(StrictModel):
    schema_: Literal["capsem.policy-pack.v1"] = Field(
        default="capsem.policy-pack.v1",
        alias="schema",
    )
    id: PackId
    version: NonEmptyStr
    status: PackStatus
    owner: PackOwner
    description: NonEmptyStr | None = None
    profile_scope: ProfileScope = Field(default_factory=ProfileScope)
    locks: PackLocks = Field(default_factory=PackLocks)
    rules: Annotated[list[PolicyRuleV1], Field(min_length=1)]

    @model_validator(mode="after")
    def _rule_ids_are_unique(self) -> "PolicyPackV1":
        seen: set[str] = set()
        for rule in self.rules:
            if rule.id in seen:
                raise ValueError(f"duplicate policy rule id: {rule.id}")
            seen.add(rule.id)
        return self


class FindingDefaults(StrictModel):
    default_severity: Severity = Severity.MEDIUM
    default_confidence: Confidence = Confidence.MEDIUM
    tags: list[NonEmptyStr] = Field(default_factory=list)
    export_routes: list[NonEmptyStr] = Field(default_factory=list)


class DetectionSourceV1(StrictModel):
    id: RuleId
    type: Literal["sigma", "ir", "reference"]
    format: Literal["yaml", "json"] | None = None
    content: NonEmptyStr | None = None
    path: NonEmptyStr | None = None
    url: NonEmptyStr | None = None
    hash: NonEmptyStr | None = None
    signature_url: NonEmptyStr | None = None

    @model_validator(mode="after")
    def _source_has_payload(self) -> "DetectionSourceV1":
        if self.content is None and self.path is None and self.url is None:
            raise ValueError("detection source requires one of content, path, or url")
        if self.type == "sigma" and self.format != "yaml":
            raise ValueError("sigma detection source requires format=yaml")
        return self


class DetectionPackV1(StrictModel):
    schema_: Literal["capsem.detection-pack.v1"] = Field(
        default="capsem.detection-pack.v1",
        alias="schema",
    )
    id: PackId
    version: NonEmptyStr
    status: PackStatus
    owner: PackOwner
    description: NonEmptyStr
    profile_scope: ProfileScope = Field(default_factory=ProfileScope)
    sources: Annotated[list[DetectionSourceV1], Field(min_length=1)]
    field_mapping: dict[NonEmptyStr, dict[NonEmptyStr, NonEmptyStr]] = Field(
        default_factory=dict,
    )
    findings: FindingDefaults = Field(default_factory=FindingDefaults)
    locks: PackLocks = Field(default_factory=PackLocks)

    @model_validator(mode="after")
    def _sigma_sources_have_mapping_and_unique_ids(self) -> "DetectionPackV1":
        seen: set[str] = set()
        needs_mapping = False
        for source in self.sources:
            if source.id in seen:
                raise ValueError(f"duplicate detection source id: {source.id}")
            seen.add(source.id)
            needs_mapping = needs_mapping or source.type == "sigma"
        if needs_mapping and not self.field_mapping:
            raise ValueError("sigma detection sources require field_mapping")
        return self


def validate_policy_pack_json(payload: str | bytes) -> PolicyPackV1:
    return PolicyPackV1.model_validate_json(payload)


def validate_detection_pack_json(payload: str | bytes) -> DetectionPackV1:
    return DetectionPackV1.model_validate_json(payload)


def validate_policy_pack_toml(path: Path) -> PolicyPackV1:
    with path.open("rb") as handle:
        raw = tomllib.load(handle)
    payload = _RawTomlAdapter.dump_json(raw)
    return PolicyPackV1.model_validate_json(payload)


def validate_detection_pack_toml(path: Path) -> DetectionPackV1:
    with path.open("rb") as handle:
        raw = tomllib.load(handle)
    payload = _RawTomlAdapter.dump_json(raw)
    return DetectionPackV1.model_validate_json(payload)


def validate_detection_pack_yaml(path: Path) -> DetectionPackV1:
    raw = yaml.safe_load(path.read_text(encoding="utf-8"))
    if not isinstance(raw, dict):
        raise ValueError("detection pack YAML must contain a mapping object")
    payload = _RawTomlAdapter.dump_json(raw)
    return DetectionPackV1.model_validate_json(payload)


def dump_policy_pack_json(pack: PolicyPackV1) -> str:
    return pack.model_dump_json(by_alias=True, exclude_none=True, indent=2)


def dump_detection_pack_json(pack: DetectionPackV1) -> str:
    return pack.model_dump_json(by_alias=True, exclude_none=True, indent=2)


def dump_policy_pack_toml(pack: PolicyPackV1) -> str:
    return tomli_w.dumps(pack.model_dump(mode="json", by_alias=True, exclude_none=True))


def dump_detection_pack_toml(pack: DetectionPackV1) -> str:
    return tomli_w.dumps(pack.model_dump(mode="json", by_alias=True, exclude_none=True))


def dump_policy_pack_schema_json() -> str:
    schema = TypeAdapter(PolicyPackV1).json_schema(
        by_alias=True,
        ref_template="#/$defs/{model}",
    )
    return TypeAdapter(dict[str, Any]).dump_json(schema, indent=2).decode()


def dump_detection_pack_schema_json() -> str:
    schema = TypeAdapter(DetectionPackV1).json_schema(
        by_alias=True,
        ref_template="#/$defs/{model}",
    )
    return TypeAdapter(dict[str, Any]).dump_json(schema, indent=2).decode()
