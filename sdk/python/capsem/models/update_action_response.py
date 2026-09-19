"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictInt, StrictStr

from .model_base import Model
from .update_action_status import UpdateActionStatus
from .update_command_plan import UpdateCommandPlan


class UpdateActionResponse(Model):
    command: UpdateCommandPlan
    exit_code: StrictInt | None = None
    status: UpdateActionStatus
    stderr: StrictStr | None = None
    stdout: StrictStr | None = None
