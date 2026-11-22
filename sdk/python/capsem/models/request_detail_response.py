"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictBool, StrictStr

from .environment_review_content import EnvironmentReviewContent
from .environment_sandbox_target import EnvironmentSandboxTarget
from .model_base import Model
from .request_decision_details import RequestDecisionDetails
from .request_summary import RequestSummary


class RequestDetailResponse(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    base: EnvironmentReviewContent | None = None
    disposition: RequestDecisionDetails | None = None
    observed_revision: StrictStr
    proposed: EnvironmentReviewContent
    restart_required: StrictBool | None = None
    summary: RequestSummary
    targets: list[EnvironmentSandboxTarget]
