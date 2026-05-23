"""Typed policy and detection pack contracts for capsem-admin."""

from __future__ import annotations

from enum import Enum
from pathlib import Path
from typing import Annotated, Any, Literal
import re
import time
import tomllib

from sigma.collection import SigmaCollection
from sigma.conditions import ConditionAND, ConditionOR
from sigma.rule.detection import SigmaDetectionItem
from sigma.types import SigmaBool, SigmaNumber, SigmaString
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


DetectionValue = str | int | float | bool


class DetectionIRMatcherV1(StrictModel):
    field_path: NonEmptyStr
    operator: Literal["equals_any"] = "equals_any"
    values: Annotated[list[DetectionValue], Field(min_length=1)]
    sigma_field: NonEmptyStr


class DetectionIRRuleV1(StrictModel):
    id: RuleId
    source_id: RuleId
    sigma_id: NonEmptyStr | None = None
    title: NonEmptyStr
    event_family: EventFamily
    condition: NonEmptyStr
    matchers: Annotated[list[DetectionIRMatcherV1], Field(min_length=1)]
    severity: Severity
    confidence: Confidence
    tags: list[NonEmptyStr] = Field(default_factory=list)


class DetectionIRV1(StrictModel):
    schema_: Literal["capsem.detection.ir.v1"] = Field(
        default="capsem.detection.ir.v1",
        alias="schema",
    )
    pack_id: PackId
    pack_version: NonEmptyStr
    pack_status: PackStatus
    owner: PackOwner
    rules: Annotated[list[DetectionIRRuleV1], Field(min_length=1)]


class SecurityEventV1(StrictModel):
    event_id: NonEmptyStr
    trace_id: NonEmptyStr | None = None
    span_id: NonEmptyStr | None = None
    timestamp: NonEmptyStr | None = None
    vm_id: NonEmptyStr | None = None
    session_id: NonEmptyStr | None = None
    profile_id: NonEmptyStr | None = None
    profile_revision: NonEmptyStr | None = None
    profile_pack_ids: list[NonEmptyStr] = Field(default_factory=list)
    user_id: NonEmptyStr | None = None
    process_id: NonEmptyStr | None = None
    parent_process_id: NonEmptyStr | None = None
    exec_id: NonEmptyStr | None = None
    turn_id: NonEmptyStr | None = None
    message_id: NonEmptyStr | None = None
    tool_call_id: NonEmptyStr | None = None
    mcp_call_id: NonEmptyStr | None = None
    event_family: EventFamily
    event_type: NonEmptyStr
    subject: dict[str, Any] = Field(default_factory=dict)
    redaction_state: Literal["raw", "redacted", "summary-only"] = "raw"


class DetectionFindingV1(StrictModel):
    event_id: NonEmptyStr
    rule_id: RuleId
    pack_id: PackId
    pack_version: NonEmptyStr
    sigma_id: NonEmptyStr | None = None
    title: NonEmptyStr
    severity: Severity
    confidence: Confidence
    tags: list[NonEmptyStr] = Field(default_factory=list)
    matched_fields: dict[NonEmptyStr, DetectionValue] = Field(default_factory=dict)


class DetectionCheckTimingV1(StrictModel):
    duration_ms: Annotated[float, Field(ge=0)]


class DetectionCheckReportV1(StrictModel):
    schema_: Literal["capsem.detection-check.v1"] = Field(
        default="capsem.detection-check.v1",
        alias="schema",
    )
    ok: bool
    pack_id: PackId
    pack_version: NonEmptyStr
    event_count: Annotated[int, Field(ge=0)]
    rule_count: Annotated[int, Field(ge=0)]
    match_count: Annotated[int, Field(ge=0)]
    findings: list[DetectionFindingV1] = Field(default_factory=list)
    diagnostics: list[str] = Field(default_factory=list)
    timing: DetectionCheckTimingV1


class PolicyBacktestMatchV1(StrictModel):
    event_ref: PolicyContextEventRefV1
    rule_id: RuleId
    pack_id: PackId
    decision: PolicyDecision
    reason: NonEmptyStr | None = None
    matched_fields: dict[NonEmptyStr, DetectionValue] = Field(default_factory=dict)


class PolicyBacktestReportV1(StrictModel):
    schema_: Literal["capsem.policy-backtest.v1"] = Field(
        default="capsem.policy-backtest.v1",
        alias="schema",
    )
    ok: bool
    pack_id: PackId
    pack_version: NonEmptyStr
    event_count: Annotated[int, Field(ge=0)]
    rule_count: Annotated[int, Field(ge=0)]
    match_count: Annotated[int, Field(ge=0)]
    rows: list[PolicyBacktestMatchV1] = Field(default_factory=list)
    diagnostics: list[str] = Field(default_factory=list)
    timing: DetectionCheckTimingV1


class PolicyCompileReportV1(StrictModel):
    schema_: Literal["capsem.policy-compile.v1"] = Field(
        default="capsem.policy-compile.v1",
        alias="schema",
    )
    ok: bool
    pack_id: PackId | None = None
    pack_version: NonEmptyStr | None = None
    rule_count: Annotated[int, Field(ge=0)]
    diagnostics: list[str] = Field(default_factory=list)


class DetectionCompileReportV1(StrictModel):
    schema_: Literal["capsem.detection-compile.v1"] = Field(
        default="capsem.detection-compile.v1",
        alias="schema",
    )
    ok: bool
    pack_id: PackId | None = None
    pack_version: NonEmptyStr | None = None
    rule_count: Annotated[int, Field(ge=0)]
    output_path: str | None = None
    diagnostics: list[str] = Field(default_factory=list)


class PolicyContextEventRefV1(StrictModel):
    corpus: NonEmptyStr
    session_id: NonEmptyStr
    event_id: NonEmptyStr
    sequence: Annotated[int, Field(ge=0)]
    timestamp_unix_ms: Annotated[int, Field(ge=0)]


class HeaderLookupV1(StrictModel):
    exists: bool
    values: list[NonEmptyStr] = Field(default_factory=list)


class BodyPolicyContextV1(StrictModel):
    state: Literal["missing", "text", "binary", "redacted"] = "missing"
    text: str | None = None
    content_type: str | None = None
    size: Annotated[int, Field(ge=0)] | None = None
    truncated: bool = False
    redaction_reason: str | None = None


class CommonPolicyContextV1(StrictModel):
    session_id: NonEmptyStr | None = None
    vm_id: NonEmptyStr | None = None
    profile_id: NonEmptyStr | None = None
    profile_revision: NonEmptyStr | None = None
    user_id: NonEmptyStr | None = None
    event_type: NonEmptyStr | None = None
    enforceability: NonEmptyStr | None = None
    actor: NonEmptyStr | None = None
    labels: dict[NonEmptyStr, NonEmptyStr] = Field(default_factory=dict)


class HttpRequestPolicyContextV1(StrictModel):
    method: NonEmptyStr | None = None
    scheme: NonEmptyStr | None = None
    host: NonEmptyStr | None = None
    port: Annotated[int, Field(ge=1, le=65535)] | None = None
    path: NonEmptyStr | None = None
    query: str | None = None
    url: NonEmptyStr | None = None
    path_class: NonEmptyStr | None = None
    bytes: Annotated[int, Field(ge=0)] | None = None
    headers: dict[NonEmptyStr, list[NonEmptyStr]] = Field(default_factory=dict)
    body: BodyPolicyContextV1 = Field(default_factory=BodyPolicyContextV1)

    def header(self, name: str) -> HeaderLookupV1:
        for key, values in self.headers.items():
            if key.lower() == name.lower():
                return HeaderLookupV1(exists=True, values=values)
        return HeaderLookupV1(exists=False)


class HttpResponsePolicyContextV1(StrictModel):
    status: Annotated[int, Field(ge=100, le=599)] | None = None
    bytes: Annotated[int, Field(ge=0)] | None = None
    headers: dict[NonEmptyStr, list[NonEmptyStr]] = Field(default_factory=dict)
    body: BodyPolicyContextV1 = Field(default_factory=BodyPolicyContextV1)

    def header(self, name: str) -> HeaderLookupV1:
        for key, values in self.headers.items():
            if key.lower() == name.lower():
                return HeaderLookupV1(exists=True, values=values)
        return HeaderLookupV1(exists=False)


class HttpPolicyContextV1(StrictModel):
    request: HttpRequestPolicyContextV1 | None = None
    response: HttpResponsePolicyContextV1 | None = None


class DnsRequestPolicyContextV1(StrictModel):
    qname: NonEmptyStr | None = None
    domain_class: NonEmptyStr | None = None


class DnsPolicyContextV1(StrictModel):
    request: DnsRequestPolicyContextV1 | None = None


class McpRequestPolicyContextV1(StrictModel):
    server_id: NonEmptyStr | None = None
    tool_name: NonEmptyStr | None = None
    namespaced_tool_name: NonEmptyStr | None = None


class McpPolicyContextV1(StrictModel):
    request: McpRequestPolicyContextV1 | None = None


class ModelRequestPolicyContextV1(StrictModel):
    provider: NonEmptyStr | None = None
    api_family: NonEmptyStr | None = None
    model: NonEmptyStr | None = None
    stream: bool | None = None


class ModelPolicyContextV1(StrictModel):
    request: ModelRequestPolicyContextV1 | None = None


class FileActivityPolicyContextV1(StrictModel):
    operation: NonEmptyStr | None = None
    path: NonEmptyStr | None = None
    path_class: NonEmptyStr | None = None


class FilePolicyContextV1(StrictModel):
    activity: FileActivityPolicyContextV1 | None = None


class ProcessActivityPolicyContextV1(StrictModel):
    operation: NonEmptyStr | None = None
    command_class: NonEmptyStr | None = None


class ProcessPolicyContextV1(StrictModel):
    activity: ProcessActivityPolicyContextV1 | None = None


class ProfileActivityPolicyContextV1(StrictModel):
    operation: NonEmptyStr | None = None
    profile_id: NonEmptyStr | None = None
    profile_revision: NonEmptyStr | None = None


class ProfilePolicyContextV1(StrictModel):
    activity: ProfileActivityPolicyContextV1 | None = None


class PolicyContextV1(StrictModel):
    schema_version: Literal[1] = 1
    common: CommonPolicyContextV1 = Field(default_factory=CommonPolicyContextV1)
    http: HttpPolicyContextV1 = Field(default_factory=HttpPolicyContextV1)
    dns: DnsPolicyContextV1 = Field(default_factory=DnsPolicyContextV1)
    mcp: McpPolicyContextV1 = Field(default_factory=McpPolicyContextV1)
    model: ModelPolicyContextV1 = Field(default_factory=ModelPolicyContextV1)
    file: FilePolicyContextV1 = Field(default_factory=FilePolicyContextV1)
    process: ProcessPolicyContextV1 = Field(default_factory=ProcessPolicyContextV1)
    profile: ProfilePolicyContextV1 = Field(default_factory=ProfilePolicyContextV1)


class PolicyContextFixtureV1(StrictModel):
    schema_: Literal["capsem.policy-context-fixture.v1"] = Field(
        default="capsem.policy-context-fixture.v1",
        alias="schema",
    )
    event_ref: PolicyContextEventRefV1
    expected_labels: list[NonEmptyStr] = Field(default_factory=list)
    context: PolicyContextV1


_LOGSOURCE_TO_EVENT_FAMILY = {
    "dns": EventFamily.DNS,
    "http": EventFamily.HTTP,
    "mcp": EventFamily.MCP,
    "model": EventFamily.MODEL,
    "file": EventFamily.FILE,
    "process": EventFamily.PROCESS,
    "credential": EventFamily.CREDENTIAL,
    "vm": EventFamily.VM,
    "profile": EventFamily.PROFILE,
    "conversation": EventFamily.CONVERSATION,
}


def _read_source_payload(source: DetectionSourceV1, base_dir: Path) -> str:
    if source.type != "sigma":
        raise ValueError(f"detection source {source.id} is not a Sigma source")
    if source.content is not None:
        return source.content
    if source.path is not None:
        source_path = Path(source.path)
        if not source_path.is_absolute():
            source_path = base_dir / source_path
        return source_path.read_text(encoding="utf-8")
    raise ValueError(f"detection source {source.id} must be embedded or local")


def _sigma_value_to_plain(value: Any) -> DetectionValue:
    if isinstance(value, SigmaString):
        if len(value.s) != 1 or not isinstance(value.s[0], str):
            raise ValueError("unsupported Sigma string wildcard or placeholder")
        return value.s[0]
    if isinstance(value, SigmaNumber):
        return value.number
    if isinstance(value, SigmaBool):
        return value.boolean
    raise ValueError(f"unsupported Sigma value type: {type(value).__name__}")


def _compile_sigma_detection_item(
    item: SigmaDetectionItem,
    *,
    source_id: str,
    event_family: EventFamily,
    field_mapping: dict[str, dict[str, str]],
) -> DetectionIRMatcherV1:
    if item.field is None:
        raise ValueError(f"source {source_id} contains unsupported keyword detection")
    if item.modifiers:
        raise ValueError(f"source {source_id} uses unsupported Sigma modifiers")
    if item.value_linking is not ConditionOR:
        raise ValueError(f"source {source_id} uses unsupported value linking")
    family_mapping = field_mapping.get(event_family.value, {})
    field_path = family_mapping.get(item.field)
    if field_path is None:
        raise ValueError(
            f"source {source_id} maps unknown Sigma field {item.field!r} "
            f"for {event_family.value}"
        )
    values = [_sigma_value_to_plain(value) for value in item.value]
    return DetectionIRMatcherV1(
        field_path=field_path,
        values=values,
        sigma_field=item.field,
    )


def _severity_from_sigma_level(level: Any, default: Severity) -> Severity:
    if level is None:
        return default
    value = getattr(level, "name", str(level)).lower()
    if value == "informational":
        return Severity.INFO
    try:
        return Severity(value)
    except ValueError:
        return default


def _rule_id_for_sigma(source_id: str, sigma_id: Any, title: str) -> str:
    suffix = str(sigma_id) if sigma_id is not None else title
    slug = re.sub(r"[^a-z0-9_.-]+", "-", suffix.lower()).strip("-_.")
    slug = slug or "rule"
    return f"{source_id}.{slug}"[:128].rstrip("-_.")


def compile_detection_pack(pack: DetectionPackV1, *, base_dir: Path) -> DetectionIRV1:
    rules: list[DetectionIRRuleV1] = []
    for source in pack.sources:
        payload = _read_source_payload(source, base_dir)
        collection = SigmaCollection.from_yaml(payload)
        for sigma_rule in collection.rules:
            product = sigma_rule.logsource.product
            category = sigma_rule.logsource.category
            if product != "capsem" or category not in _LOGSOURCE_TO_EVENT_FAMILY:
                raise ValueError(
                    f"source {source.id} has unsupported Sigma logsource "
                    f"{product!r}/{category!r}"
                )
            event_family = _LOGSOURCE_TO_EVENT_FAMILY[category]
            conditions = sigma_rule.detection.condition
            if len(conditions) != 1:
                raise ValueError(f"source {source.id} has unsupported Sigma condition")
            selection_name = conditions[0].strip()
            if selection_name not in sigma_rule.detection.detections:
                raise ValueError(f"source {source.id} has unsupported Sigma condition")
            detection = sigma_rule.detection.detections[selection_name]
            if detection.item_linking is not ConditionAND:
                raise ValueError(f"source {source.id} uses unsupported item linking")
            matchers = [
                _compile_sigma_detection_item(
                    item,
                    source_id=source.id,
                    event_family=event_family,
                    field_mapping=pack.field_mapping,
                )
                for item in detection.detection_items
            ]
            rule_id = source.id
            if len(collection.rules) > 1:
                rule_id = _rule_id_for_sigma(
                    source.id,
                    sigma_rule.id,
                    sigma_rule.title,
                )
            rules.append(
                DetectionIRRuleV1(
                    id=rule_id,
                    source_id=source.id,
                    sigma_id=str(sigma_rule.id) if sigma_rule.id is not None else None,
                    title=sigma_rule.title,
                    event_family=event_family,
                    condition=selection_name,
                    matchers=matchers,
                    severity=_severity_from_sigma_level(
                        sigma_rule.level,
                        pack.findings.default_severity,
                    ),
                    confidence=pack.findings.default_confidence,
                    tags=pack.findings.tags + [str(tag) for tag in sigma_rule.tags],
                )
            )
    return DetectionIRV1(
        pack_id=pack.id,
        pack_version=pack.version,
        pack_status=pack.status,
        owner=pack.owner,
        rules=rules,
    )


def _event_value(context: PolicyContextV1, field_path: str) -> DetectionValue | None:
    current: Any = context.model_dump(mode="json")
    for part in field_path.split("."):
        if not isinstance(current, dict) or part not in current:
            return None
        current = current[part]
    if isinstance(current, str | int | float | bool):
        return current
    return None


def _rule_matches(
    rule: DetectionIRRuleV1,
    fixture: PolicyContextFixtureV1,
) -> tuple[bool, dict[str, DetectionValue]]:
    if _context_event_family(fixture.context) is not rule.event_family:
        return False, {}
    matched: dict[str, DetectionValue] = {}
    for matcher in rule.matchers:
        value = _event_value(fixture.context, matcher.field_path)
        if value is None or value not in matcher.values:
            return False, {}
        matched[matcher.field_path] = value
    return True, matched


_STRING_LITERAL = r"(?P<quote>['\"])(?P<value>[^'\"]*)(?P=quote)"
_CONTAINS_RE = re.compile(
    rf"^(?P<path>[a-z][a-z0-9_.]*)\.contains\({_STRING_LITERAL}\)$"
)
_STARTS_WITH_RE = re.compile(
    rf"^(?P<path>[a-z][a-z0-9_.]*)\.startsWith\({_STRING_LITERAL}\)$"
)
_EQUALS_RE = re.compile(
    rf"^(?P<path>[a-z][a-z0-9_.]*)\s*==\s*{_STRING_LITERAL}$"
)
_HEADER_EXISTS_RE = re.compile(
    r"^http\.request\.header\((?P<quote>['\"])(?P<header>[^'\"]+)(?P=quote)\)\.exists\(\)$",
)

_SUPPORTED_POLICY_PATHS: dict[EventFamily, frozenset[str]] = {
    EventFamily.DNS: frozenset(
        {
            "dns.request.qname",
            "dns.request.domain_class",
        }
    ),
    EventFamily.HTTP: frozenset(
        {
            "http.request.method",
            "http.request.scheme",
            "http.request.host",
            "http.request.port",
            "http.request.path",
            "http.request.query",
            "http.request.url",
            "http.request.path_class",
            "http.request.bytes",
            "http.request.body.state",
            "http.request.body.text",
            "http.request.body.content_type",
            "http.request.body.size",
            "http.request.body.truncated",
            "http.request.body.redaction_reason",
            "http.response.status",
            "http.response.bytes",
            "http.response.body.state",
            "http.response.body.text",
            "http.response.body.content_type",
            "http.response.body.size",
            "http.response.body.truncated",
            "http.response.body.redaction_reason",
        }
    ),
    EventFamily.MCP: frozenset(
        {
            "mcp.request.server_id",
            "mcp.request.tool_name",
            "mcp.request.namespaced_tool_name",
        }
    ),
    EventFamily.MODEL: frozenset(
        {
            "model.request.provider",
            "model.request.api_family",
            "model.request.model",
            "model.request.stream",
        }
    ),
    EventFamily.FILE: frozenset(
        {
            "file.activity.operation",
            "file.activity.path",
            "file.activity.path_class",
        }
    ),
    EventFamily.PROCESS: frozenset(
        {
            "process.activity.operation",
            "process.activity.command_class",
        }
    ),
    EventFamily.PROFILE: frozenset(
        {
            "profile.activity.operation",
            "profile.activity.profile_id",
            "profile.activity.profile_revision",
        }
    ),
}


def _validate_policy_path(rule: PolicyRuleV1, path: str) -> None:
    family_paths = _SUPPORTED_POLICY_PATHS.get(rule.event_family, frozenset())
    if path in family_paths:
        return
    raise ValueError(
        f"rule {rule.id} uses unsupported policy path for "
        f"{rule.event_family.value}: {path}"
    )


def _validate_policy_condition(rule: PolicyRuleV1) -> None:
    for clause in (part.strip() for part in rule.condition.split("&&")):
        if not clause:
            continue
        if clause in {"event", "subject"} or clause.startswith(("event.", "subject.")):
            raise ValueError(f"rule {rule.id} uses unsupported internal event root")
        if clause in {"false", "true"}:
            continue
        if _HEADER_EXISTS_RE.match(clause) is not None:
            if rule.event_family is not EventFamily.HTTP:
                raise ValueError(
                    f"rule {rule.id} uses unsupported policy path for "
                    f"{rule.event_family.value}: http.request.header"
                )
            continue
        contains_match = _CONTAINS_RE.match(clause)
        if contains_match is not None:
            _validate_policy_path(rule, contains_match.group("path"))
            continue
        starts_with_match = _STARTS_WITH_RE.match(clause)
        if starts_with_match is not None:
            _validate_policy_path(rule, starts_with_match.group("path"))
            continue
        equals_match = _EQUALS_RE.match(clause)
        if equals_match is not None:
            _validate_policy_path(rule, equals_match.group("path"))
            continue
        raise ValueError(f"rule {rule.id} uses unsupported CEL clause: {clause}")


def _policy_rule_matches(
    rule: PolicyRuleV1,
    fixture: PolicyContextFixtureV1,
) -> tuple[bool, dict[str, DetectionValue]]:
    if _context_event_family(fixture.context) is not rule.event_family:
        return False, {}
    matched: dict[str, DetectionValue] = {}
    _validate_policy_condition(rule)
    for clause in (part.strip() for part in rule.condition.split("&&")):
        if not clause:
            continue
        if clause == "false":
            return False, {}
        if clause == "true":
            matched["condition"] = True
            continue
        header_match = _HEADER_EXISTS_RE.match(clause)
        if header_match is not None:
            request = fixture.context.http.request
            if request is None:
                return False, {}
            lookup = request.header(header_match.group("header"))
            if not lookup.exists:
                return False, {}
            matched[f"http.request.headers.{header_match.group('header').lower()}"] = (
                lookup.values[0] if lookup.values else True
            )
            continue
        contains_match = _CONTAINS_RE.match(clause)
        if contains_match is not None:
            path = contains_match.group("path")
            value = _event_value(fixture.context, path)
            needle = contains_match.group("value")
            if not isinstance(value, str) or needle not in value:
                return False, {}
            matched[path] = value
            continue
        starts_with_match = _STARTS_WITH_RE.match(clause)
        if starts_with_match is not None:
            path = starts_with_match.group("path")
            value = _event_value(fixture.context, path)
            prefix = starts_with_match.group("value")
            if not isinstance(value, str) or not value.startswith(prefix):
                return False, {}
            matched[path] = value
            continue
        equals_match = _EQUALS_RE.match(clause)
        if equals_match is not None:
            path = equals_match.group("path")
            value = _event_value(fixture.context, path)
            expected = equals_match.group("value")
            if value != expected:
                return False, {}
            matched[path] = value
            continue
        raise ValueError(f"rule {rule.id} uses unsupported CEL clause: {clause}")
    return bool(matched), matched


def _context_event_family(context: PolicyContextV1) -> EventFamily | None:
    event_type = context.common.event_type
    if event_type is None or "." not in event_type:
        return None
    family = event_type.split(".", 1)[0]
    try:
        return EventFamily(family)
    except ValueError:
        return None


def run_detection_check(
    pack: DetectionPackV1,
    *,
    events_path: Path,
    base_dir: Path,
) -> DetectionCheckReportV1:
    started = time.perf_counter()
    diagnostics: list[str] = []
    findings: list[DetectionFindingV1] = []
    ir = compile_detection_pack(pack, base_dir=base_dir)
    try:
        fixtures = load_policy_context_fixture_jsonl(events_path)
    except Exception as error:
        fixtures = []
        diagnostics.append(str(error))
    for fixture in fixtures:
        for rule in ir.rules:
            matched, matched_fields = _rule_matches(rule, fixture)
            if matched:
                findings.append(
                    DetectionFindingV1(
                        event_id=fixture.event_ref.event_id,
                        rule_id=rule.id,
                        pack_id=ir.pack_id,
                        pack_version=ir.pack_version,
                        sigma_id=rule.sigma_id,
                        title=rule.title,
                        severity=rule.severity,
                        confidence=rule.confidence,
                        tags=rule.tags,
                        matched_fields=matched_fields,
                    )
                )
    return DetectionCheckReportV1(
        ok=not diagnostics,
        pack_id=pack.id,
        pack_version=pack.version,
        event_count=len(fixtures),
        rule_count=len(ir.rules),
        match_count=len(findings),
        findings=findings,
        diagnostics=diagnostics,
        timing=DetectionCheckTimingV1(
            duration_ms=(time.perf_counter() - started) * 1000,
        ),
    )


def compile_policy_pack(pack: PolicyPackV1) -> PolicyCompileReportV1:
    diagnostics: list[str] = []
    for rule in pack.rules:
        try:
            _validate_policy_condition(rule)
        except Exception as error:
            diagnostics.append(str(error))
    return PolicyCompileReportV1(
        ok=not diagnostics,
        pack_id=pack.id,
        pack_version=pack.version,
        rule_count=len(pack.rules),
        diagnostics=diagnostics,
    )


def run_policy_backtest(
    pack: PolicyPackV1,
    *,
    events_path: Path,
) -> PolicyBacktestReportV1:
    started = time.perf_counter()
    compile_report = compile_policy_pack(pack)
    diagnostics: list[str] = list(compile_report.diagnostics)
    rows: list[PolicyBacktestMatchV1] = []
    try:
        fixtures = load_policy_context_fixture_jsonl(events_path)
    except Exception as error:
        fixtures = []
        diagnostics.append(str(error))
    if compile_report.ok:
        for fixture in fixtures:
            for rule in sorted(pack.rules, key=lambda item: (item.priority, item.id)):
                if not rule.enabled:
                    continue
                try:
                    matched, matched_fields = _policy_rule_matches(rule, fixture)
                except Exception as error:
                    diagnostics.append(str(error))
                    continue
                if matched:
                    rows.append(
                        PolicyBacktestMatchV1(
                            event_ref=fixture.event_ref,
                            rule_id=rule.id,
                            pack_id=pack.id,
                            decision=rule.decision,
                            reason=rule.reason,
                            matched_fields=matched_fields,
                        )
                    )
                    break
    return PolicyBacktestReportV1(
        ok=not diagnostics,
        pack_id=pack.id,
        pack_version=pack.version,
        event_count=len(fixtures),
        rule_count=len(pack.rules),
        match_count=len(rows),
        rows=rows,
        diagnostics=diagnostics,
        timing=DetectionCheckTimingV1(
            duration_ms=(time.perf_counter() - started) * 1000,
        ),
    )


def dump_detection_ir_json(ir: DetectionIRV1) -> str:
    return ir.model_dump_json(by_alias=True, exclude_none=True, indent=2)


def dump_policy_compile_report_json(report: PolicyCompileReportV1) -> str:
    return report.model_dump_json(by_alias=True, exclude_none=True, indent=2)


def dump_policy_backtest_report_json(report: PolicyBacktestReportV1) -> str:
    return report.model_dump_json(by_alias=True, exclude_none=True, indent=2)


def dump_detection_check_report_json(report: DetectionCheckReportV1) -> str:
    return report.model_dump_json(by_alias=True, exclude_none=True, indent=2)


def dump_detection_compile_report_json(report: DetectionCompileReportV1) -> str:
    return report.model_dump_json(by_alias=True, exclude_none=True, indent=2)


def load_policy_context_fixture_jsonl(path: Path) -> list[PolicyContextFixtureV1]:
    fixtures: list[PolicyContextFixtureV1] = []
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip():
            continue
        try:
            fixtures.append(PolicyContextFixtureV1.model_validate_json(line))
        except Exception as error:
            raise ValueError(f"{path}:{line_number}: {error}") from error
    return fixtures


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


def dump_detection_ir_schema_json() -> str:
    schema = TypeAdapter(DetectionIRV1).json_schema(
        by_alias=True,
        ref_template="#/$defs/{model}",
    )
    return TypeAdapter(dict[str, Any]).dump_json(schema, indent=2).decode()
