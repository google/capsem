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


def _event_value(event: SecurityEventV1, field_path: str) -> DetectionValue | None:
    current: Any = event.model_dump(mode="json")
    for part in field_path.split("."):
        if not isinstance(current, dict) or part not in current:
            return None
        current = current[part]
    if isinstance(current, str | int | float | bool):
        return current
    return None


def _rule_matches(
    rule: DetectionIRRuleV1,
    event: SecurityEventV1,
) -> tuple[bool, dict[str, DetectionValue]]:
    if event.event_family is not rule.event_family:
        return False, {}
    matched: dict[str, DetectionValue] = {}
    for matcher in rule.matchers:
        value = _event_value(event, matcher.field_path)
        if value is None or value not in matcher.values:
            return False, {}
        matched[matcher.field_path] = value
    return True, matched


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
    event_count = 0
    for line_number, line in enumerate(events_path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip():
            continue
        try:
            event = SecurityEventV1.model_validate_json(line)
        except Exception as error:
            diagnostics.append(f"{events_path}:{line_number}: {error}")
            continue
        event_count += 1
        for rule in ir.rules:
            matched, matched_fields = _rule_matches(rule, event)
            if matched:
                findings.append(
                    DetectionFindingV1(
                        event_id=event.event_id,
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
        event_count=event_count,
        rule_count=len(ir.rules),
        match_count=len(findings),
        findings=findings,
        diagnostics=diagnostics,
        timing=DetectionCheckTimingV1(
            duration_ms=(time.perf_counter() - started) * 1000,
        ),
    )


def dump_detection_ir_json(ir: DetectionIRV1) -> str:
    return ir.model_dump_json(by_alias=True, exclude_none=True, indent=2)


def dump_detection_check_report_json(report: DetectionCheckReportV1) -> str:
    return report.model_dump_json(by_alias=True, exclude_none=True, indent=2)


def dump_detection_compile_report_json(report: DetectionCompileReportV1) -> str:
    return report.model_dump_json(by_alias=True, exclude_none=True, indent=2)


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
