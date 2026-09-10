"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from .audit_event import AuditEvent
from .credential_event import CredentialEvent
from .dns_event import DnsEvent
from .event_body import EventBody
from .file_event import FileEvent
from .http_event import HttpEvent
from .interaction_report import InteractionReport
from .model_base import Model
from .model_event import ModelEvent
from .model_usage import ModelUsage
from .process_event import ProcessEvent
from .tool_event import ToolEvent


class VmStatsDetailResponse(Model):
    audit_events: list[AuditEvent]
    body_blobs: dict[str, list[EventBody]]
    credential_events: list[CredentialEvent]
    dns_events: list[DnsEvent]
    file_events: list[FileEvent]
    http_events: list[HttpEvent]
    interactions: InteractionReport
    model_events: list[ModelEvent]
    model_stats: list[ModelUsage]
    process_events: list[ProcessEvent]
    tool_events: list[ToolEvent]
