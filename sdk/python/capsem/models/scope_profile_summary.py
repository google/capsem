"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictBool, StrictStr

from .capability_summary import CapabilitySummary
from .enrollment_kind import EnrollmentKind
from .model_base import Model


class ScopeProfileSummary(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    application_label: StrictStr
    capabilities: list[CapabilitySummary]
    enabled: StrictBool
    enrollment_kind: EnrollmentKind
    id: StrictStr
    label: StrictStr
    requested_scopes: list[StrictStr]
